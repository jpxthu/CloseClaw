//! Dreaming — three-stage memory promotion pipeline.
//!
//! Consumes structured memory entries produced by the memory-miner and
//! promotes high-value ones to MEMORY.md through Light → REM → Deep stages.
//!
//! All three stages are **programmatic** (no LLM calls).

use thiserror::Error;

use crate::session::persistence::{DreamingStatus, PersistenceError, PersistenceService};

// ── Types ────────────────────────────────────────────────────────────────

/// A structured memory entry produced by the memory-miner.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEntry {
    /// Entry category.
    pub category: EntryCategory,
    /// Human-readable body text.
    pub body: String,
    /// When the entry was produced.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Source session that generated this entry.
    pub source_session_id: String,
    /// Concept tags assigned during the REM stage.
    tags: Vec<String>,
    /// Aggregate score from the Deep stage.
    score: f64,
}

/// Memory entry category (matches design doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryCategory {
    /// User preference (e.g. "always use dark mode").
    Preference,
    /// A decision made during a session.
    Decision,
    /// A lesson learned.
    Lesson,
    /// A factual piece of information.
    Fact,
}

/// Errors specific to the dreaming pipeline.
#[derive(Debug, Error)]
pub enum DreamingError {
    /// Storage layer error.
    #[error("storage error: {0}")]
    Storage(#[from] PersistenceError),

    /// An I/O error occurred while writing MEMORY.md.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The pipeline encountered corrupted or invalid data.
    #[error("data error: {0}")]
    Data(String),
}

// ── Scoring weights (Deep stage) ─────────────────────────────────────────

/// Configurable scoring weights for the Deep stage.
#[derive(Debug, Clone)]
struct ScoringWeights {
    /// Weight for frequency (similar info across multiple sessions).
    frequency: f64,
    /// Weight for timeliness (recency bonus).
    timeliness: f64,
    /// Weight for clarity (owner-explicit vs agent-inferred).
    clarity: f64,
    /// Weight for persistence (decision/preference vs temporary fact).
    persistence: f64,
    /// Weight for relevance to existing MEMORY.md content.
    relevance: f64,
    /// Penalty weight for negative signals.
    negative: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            frequency: 1.0,
            timeliness: 0.8,
            clarity: 1.2,
            persistence: 1.5,
            relevance: 0.6,
            negative: -2.0,
        }
    }
}

/// Thresholds for the Deep stage three-gate filtering.
#[derive(Debug, Clone)]
struct Thresholds {
    /// Absolute minimum score; entries below this are discarded.
    absolute_min: f64,
    /// Relative minimum within the same category; entries scoring
    /// below this fraction of the top entry are discarded.
    relative_min_ratio: f64,
    /// Maximum number of entries in the final MEMORY.md.
    capacity: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            absolute_min: 0.3,
            relative_min_ratio: 0.2,
            capacity: 200,
        }
    }
}

// ── DreamingPipeline ─────────────────────────────────────────────────────

/// Orchestrates the three-stage dreaming pipeline.
pub struct DreamingPipeline {
    weights: ScoringWeights,
    thresholds: Thresholds,
}

impl DreamingPipeline {
    /// Create a pipeline with default weights and thresholds.
    pub fn new() -> Self {
        Self {
            weights: ScoringWeights::default(),
            thresholds: Thresholds::default(),
        }
    }

    /// Execute one full dreaming cycle.
    ///
    /// Reads mined-but-undreamt sessions from `storage`, processes them
    /// through Light → REM → Deep, and writes surviving entries to
    /// MEMORY.md.
    pub async fn run_once(&self, storage: &dyn PersistenceService) -> Result<(), DreamingError> {
        let session_ids = storage.list_mined_undreamt_sessions().await?;
        if session_ids.is_empty() {
            return Ok(());
        }

        let mut all_entries = Vec::new();
        for sid in &session_ids {
            let entries = self.collect_entries_for_session(storage, sid).await?;
            all_entries.extend(entries);
        }

        if all_entries.is_empty() {
            return Ok(());
        }

        let light = self.light_stage(all_entries)?;
        let rem = self.rem_stage(light);
        let deep = self.deep_stage(rem);

        for sid in &session_ids {
            storage
                .update_dreaming_status(sid, DreamingStatus::Completed)
                .await?;
        }

        let _ = deep; // Placeholder: actual MEMORY.md write happens in integration.
        Ok(())
    }

    /// Collect unprocessed entries for a single session.
    async fn collect_entries_for_session(
        &self,
        storage: &dyn PersistenceService,
        session_id: &str,
    ) -> Result<Vec<MemoryEntry>, DreamingError> {
        storage
            .update_dreaming_status(session_id, DreamingStatus::InLight)
            .await?;
        // TODO: load actual entries from memory store
        let _ = (storage, session_id);
        Ok(Vec::new())
    }

    // ── Light stage ──────────────────────────────────────────────────

    /// Light stage: deduplicate and chunk entries by source session.
    fn light_stage(
        &self,
        entries: Vec<MemoryEntry>,
    ) -> Result<Vec<Vec<MemoryEntry>>, DreamingError> {
        let deduped = self.deduplicate(entries);
        Ok(self.chunk_by_session(deduped))
    }

    /// Remove entries that are duplicates (same category + source + high
    /// body similarity).
    fn deduplicate(&self, entries: Vec<MemoryEntry>) -> Vec<MemoryEntry> {
        let mut seen = std::collections::HashSet::new();
        entries
            .into_iter()
            .filter(|e| {
                let key = (e.category, e.source_session_id.clone(), e.body.clone());
                seen.insert(key)
            })
            .collect()
    }

    /// Split entries into groups by source session ID.
    fn chunk_by_session(&self, entries: Vec<MemoryEntry>) -> Vec<Vec<MemoryEntry>> {
        let mut groups: std::collections::HashMap<String, Vec<MemoryEntry>> =
            std::collections::HashMap::new();
        for e in entries {
            groups
                .entry(e.source_session_id.clone())
                .or_default()
                .push(e);
        }
        groups.into_values().collect()
    }

    // ── REM stage ────────────────────────────────────────────────────

    /// REM stage: statistical extraction and concept tag aggregation.
    fn rem_stage(&self, chunks: Vec<Vec<MemoryEntry>>) -> Vec<MemoryEntry> {
        let mut all_entries: Vec<MemoryEntry> = chunks.into_iter().flatten().collect();
        self.aggregate_tags(&mut all_entries);
        all_entries
    }

    /// Assign concept tags to entries based on keyword co-occurrence.
    fn aggregate_tags(&self, entries: &mut [MemoryEntry]) {
        let mut word_freq: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for e in entries.iter() {
            for word in Self::extract_words(&e.body) {
                *word_freq.entry(word).or_insert(0) += 1;
            }
        }

        let top_words: Vec<String> = {
            let mut pairs: Vec<_> = word_freq.into_iter().collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1));
            pairs.into_iter().take(10).map(|(w, _)| w).collect()
        };

        for e in entries.iter_mut() {
            let body_words: std::collections::HashSet<String> =
                Self::extract_words(&e.body).into_iter().collect();
            e.tags = top_words
                .iter()
                .filter(|w| body_words.contains(*w))
                .cloned()
                .collect();
        }
    }

    /// Split body text into lowercase words for keyword extraction.
    fn extract_words(body: &str) -> Vec<String> {
        body.split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() > 2)
            .collect()
    }

    // ── Deep stage ───────────────────────────────────────────────────

    /// Deep stage: score each entry, apply three thresholds, return
    /// survivors.
    fn deep_stage(&self, entries: Vec<MemoryEntry>) -> Vec<MemoryEntry> {
        let mut scored: Vec<MemoryEntry> =
            entries.into_iter().map(|e| self.score_entry(e)).collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Gate 1: absolute threshold
        scored.retain(|e| e.score >= self.thresholds.absolute_min);

        // Gate 2: relative threshold (per-category)
        self.apply_relative_filter(&mut scored);

        // Gate 3: capacity limit
        scored.truncate(self.thresholds.capacity);

        scored
    }

    /// Compute a weighted score for a single entry.
    fn score_entry(&self, mut entry: MemoryEntry) -> MemoryEntry {
        let age_hours = (chrono::Utc::now() - entry.timestamp).num_hours().max(0) as f64;
        let timeliness = 1.0 / (1.0 + age_hours / 168.0); // half-life ≈ 1 week

        let clarity = match entry.category {
            EntryCategory::Preference | EntryCategory::Decision => 1.0,
            EntryCategory::Lesson | EntryCategory::Fact => 0.6,
        };

        let persistence = match entry.category {
            EntryCategory::Preference | EntryCategory::Decision => 1.0,
            _ => 0.4,
        };

        let frequency = 1.0; // TODO: compute from cross-session duplicates
        let relevance = entry.tags.len() as f64 / 10.0;
        let negative = 0.0; // TODO: detect conflicting info

        let w = &self.weights;
        entry.score = w.frequency * frequency
            + w.timeliness * timeliness
            + w.clarity * clarity
            + w.persistence * persistence
            + w.relevance * relevance
            + w.negative * negative;

        entry
    }

    /// Remove entries scoring below `relative_min_ratio × top_score`
    /// within the same category.
    fn apply_relative_filter(&self, entries: &mut Vec<MemoryEntry>) {
        if entries.is_empty() {
            return;
        }

        let categories: Vec<EntryCategory> = entries.iter().map(|e| e.category).collect();
        let unique_cats: std::collections::HashSet<EntryCategory> =
            categories.into_iter().collect();

        for cat in unique_cats {
            let cat_scores: Vec<f64> = entries
                .iter()
                .filter(|e| e.category == cat)
                .map(|e| e.score)
                .collect();
            if cat_scores.is_empty() {
                continue;
            }
            let max_score = cat_scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let cutoff = max_score * self.thresholds.relative_min_ratio;

            entries.retain(|e| e.category != cat || e.score >= cutoff);
        }
    }
}

impl Default for DreamingPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_entry(
        category: EntryCategory,
        body: &str,
        session_id: &str,
        minutes_ago: i64,
    ) -> MemoryEntry {
        MemoryEntry {
            category,
            body: body.to_string(),
            timestamp: chrono::Utc::now() - chrono::Duration::minutes(minutes_ago),
            source_session_id: session_id.to_string(),
            tags: Vec::new(),
            score: 0.0,
        }
    }

    #[test]
    fn test_light_dedup_removes_duplicates() {
        let pipeline = DreamingPipeline::new();
        let entries = vec![
            make_entry(EntryCategory::Fact, "dark mode preferred", "s1", 10),
            make_entry(EntryCategory::Fact, "dark mode preferred", "s1", 10),
            make_entry(EntryCategory::Fact, "light theme is nice", "s1", 5),
        ];
        let result = pipeline.deduplicate(entries);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_light_chunk_by_session() {
        let pipeline = DreamingPipeline::new();
        let entries = vec![
            make_entry(EntryCategory::Fact, "a", "s1", 10),
            make_entry(EntryCategory::Fact, "b", "s2", 10),
            make_entry(EntryCategory::Fact, "c", "s1", 5),
        ];
        let chunks = pipeline.chunk_by_session(entries);
        assert_eq!(chunks.len(), 2);
        let s1: Vec<_> = chunks
            .iter()
            .filter(|c| c[0].source_session_id == "s1")
            .collect();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].len(), 2);
    }

    #[test]
    fn test_deep_scoring_thresholds() {
        let pipeline = DreamingPipeline::new();
        let entries = vec![
            make_entry(EntryCategory::Preference, "always use vim", "s1", 10),
            make_entry(EntryCategory::Fact, "the sky is blue", "s1", 10),
        ];
        let result = pipeline.deep_stage(entries);
        // Preference gets higher score than Fact due to clarity/persistence
        assert!(!result.is_empty());
    }

    #[test]
    fn test_deep_capacity_limit() {
        let pipeline = DreamingPipeline::new();
        let mut entries = Vec::new();
        for i in 0..300 {
            entries.push(make_entry(
                EntryCategory::Fact,
                &format!("fact number {i}"),
                "s1",
                i,
            ));
        }
        let result = pipeline.deep_stage(entries);
        assert!(result.len() <= 200);
    }

    #[test]
    fn test_extract_words_filters_short() {
        let words = DreamingPipeline::extract_words("I am okay to go now");
        assert!(!words.contains(&"i".to_string()));
        assert!(words.contains(&"okay".to_string()));
    }
}
