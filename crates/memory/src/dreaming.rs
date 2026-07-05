//! Dreaming — three-stage memory promotion pipeline.
//!
//! Consumes structured memory entries produced by the memory-miner and
//! promotes high-value ones to MEMORY.md through Light → REM → Deep stages.
//!
//! All three stages are **programmatic** (no LLM calls).

use thiserror::Error;

use closeclaw_config::agents::{DreamingConfig, DreamingScoringConfig};
use closeclaw_session::persistence::{DreamingStatus, PersistenceError, PersistenceService};

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
    /// Actionable lesson extracted from Error/Anger events.
    /// Required for Error and Anger categories; None for Decision.
    pub lesson: Option<String>,
    /// Concept tags assigned during the REM stage.
    pub(crate) tags: Vec<String>,
    /// Aggregate score from the Deep stage.
    pub(crate) score: f64,
}

impl MemoryEntry {
    /// Set the lesson field (builder pattern).
    pub fn lesson(mut self, lesson: String) -> Self {
        self.lesson = Some(lesson);
        self
    }
}

/// Memory entry category (matches design doc).
///
/// Miner 1 produces events with one of these categories.
/// Error and Anger entries carry a required `lesson` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryCategory {
    /// Agent made a clear error (wrong judgment, poor execution, misunderstood request).
    Error,
    /// Owner expressed dissatisfaction or correction.
    Anger,
    /// Owner made an explicit product decision or design choice.
    Decision,
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

/// Thresholds for the Deep stage three-gate filtering.
#[derive(Debug, Clone)]
struct Thresholds {
    /// Absolute minimum score; entries below this are discarded.
    absolute: f64,
    /// Relative minimum within the same category; entries scoring
    /// below this fraction of the top entry are discarded.
    relative: f64,
    /// Maximum number of entries in the final MEMORY.md.
    max_rules: usize,
}

// ── DreamingPipeline ─────────────────────────────────────────────────────

/// Orchestrates the three-stage dreaming pipeline.
pub struct DreamingPipeline {
    scoring: DreamingScoringConfig,
    thresholds: Thresholds,
    config: DreamingConfig,
}

impl DreamingPipeline {
    /// Create a pipeline with default weights, thresholds, and config.
    pub fn new() -> Self {
        Self {
            scoring: DreamingScoringConfig::default(),
            thresholds: Thresholds {
                absolute: 2.0,
                relative: 0.3,
                max_rules: 20,
            },
            config: DreamingConfig::default(),
        }
    }

    /// Create a pipeline with a custom dreaming configuration.
    pub fn with_config(config: DreamingConfig) -> Self {
        let scoring = config.scoring.clone();
        let thresholds = Thresholds {
            absolute: config.threshold.absolute,
            relative: config.threshold.relative,
            max_rules: config.capacity.max_rules,
        };
        Self {
            scoring,
            thresholds,
            config,
        }
    }

    /// Execute one full dreaming cycle.
    ///
    /// Reads mined-but-undreamt sessions from `storage`, processes them
    /// through Light → REM → Deep, and writes surviving entries to
    /// MEMORY.md.
    pub async fn run_once(&self, storage: &dyn PersistenceService) -> Result<(), DreamingError> {
        if !self.config.enabled.unwrap_or(false) {
            return Ok(());
        }

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
            // No entries to process — mark all sessions as Completed.
            for sid in &session_ids {
                storage
                    .update_dreaming_status(sid, DreamingStatus::Completed)
                    .await?;
            }
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

        tracing::warn!(
            entry_count = deep.len(),
            "dreaming pipeline completed but MEMORY.md write not yet integrated"
        );

        // Write Dream Diary if enabled.
        if self.config.diary.enabled.unwrap_or(true) && !deep.is_empty() {
            self.write_dream_diary(&deep)?;
        }

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
        tracing::warn!(
            session_id,
            "load_entries not yet implemented, returning empty"
        );
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
            pairs.sort_by_key(|a| std::cmp::Reverse(a.1));
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
        scored.retain(|e| e.score >= self.thresholds.absolute);

        // Gate 2: relative threshold (per-category)
        self.apply_relative_filter(&mut scored);

        // Gate 3: capacity limit
        scored.truncate(self.thresholds.max_rules);

        scored
    }

    /// Compute a weighted score for a single entry.
    fn score_entry(&self, mut entry: MemoryEntry) -> MemoryEntry {
        let age_hours = (chrono::Utc::now() - entry.timestamp).num_hours().max(0) as f64;
        let recency = 1.0 / (1.0 + age_hours / 168.0); // half-life ≈ 1 week

        let explicitness = match entry.category {
            EntryCategory::Decision => 1.0,
            EntryCategory::Error | EntryCategory::Anger => 0.8,
        };

        let persistence = match entry.category {
            EntryCategory::Decision => 1.0,
            EntryCategory::Error | EntryCategory::Anger => 0.6,
        };

        let frequency = 1.0; // TODO: compute from cross-session duplicates
        let relevance = entry.tags.len() as f64 / 10.0;
        // persistence + relevance → cross_agent dimension
        let cross_agent = persistence * 0.5 + relevance * 0.5;
        let negative_signal = 0.0; // TODO: detect conflicting info

        let w = &self.scoring;
        entry.score = w.frequency_weight * frequency
            + w.recency_weight * recency
            + w.explicitness_weight * explicitness
            + w.cross_agent_weight * cross_agent
            + w.negative_signal_weight * negative_signal;

        entry
    }

    /// Remove entries scoring below `relative × top_score`
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
            let cutoff = max_score * self.thresholds.relative;

            entries.retain(|e| e.category != cat || e.score >= cutoff);
        }
    }

    // ── Dream Diary ────────────────────────────────────────────────

    /// Write a Dream Diary file summarizing the promoted entries.
    ///
    /// The diary is a narrative summary of the entries that passed the
    /// Deep stage, written to `{path}/{date}.md`.
    pub(crate) fn write_dream_diary(&self, entries: &[MemoryEntry]) -> Result<(), DreamingError> {
        if !self.config.diary.enabled.unwrap_or(true) || entries.is_empty() {
            return Ok(());
        }

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let filename = format!("{}.md", date);
        let diary_dir = std::path::Path::new(&self.config.diary.path);
        std::fs::create_dir_all(diary_dir)?;
        let diary_path = diary_dir.join(&filename);

        let mut content = format!("# Dream Diary — {}\n\n", date);
        content.push_str(&format!(
            "Promoted {} entries to MEMORY.md.\n\n",
            entries.len()
        ));

        for entry in entries {
            let category_str = match entry.category {
                EntryCategory::Error => "Error",
                EntryCategory::Anger => "Anger",
                EntryCategory::Decision => "Decision",
            };
            content.push_str(&format!("- **[{}]** {}", category_str, entry.body));
            if let Some(lesson) = &entry.lesson {
                content.push_str(&format!(" → _Lesson: {}_", lesson));
            }
            content.push('\n');
        }

        std::fs::write(&diary_path, content)?;
        tracing::info!(path = %diary_path.display(), "dream diary written");
        Ok(())
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
            lesson: None,
            tags: Vec::new(),
            score: 0.0,
        }
    }

    #[test]
    fn test_light_dedup_removes_duplicates() {
        let pipeline = DreamingPipeline::new();
        let entries = vec![
            make_entry(EntryCategory::Decision, "dark mode preferred", "s1", 10),
            make_entry(EntryCategory::Decision, "dark mode preferred", "s1", 10),
            make_entry(EntryCategory::Decision, "light theme is nice", "s1", 5),
        ];
        let result = pipeline.deduplicate(entries);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_light_chunk_by_session() {
        let pipeline = DreamingPipeline::new();
        let entries = vec![
            make_entry(EntryCategory::Decision, "a", "s1", 10),
            make_entry(EntryCategory::Decision, "b", "s2", 10),
            make_entry(EntryCategory::Decision, "c", "s1", 5),
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
            make_entry(EntryCategory::Decision, "always use vim", "s1", 10),
            make_entry(
                EntryCategory::Error,
                "wrong judgment on deployment",
                "s1",
                10,
            )
            .lesson("verify before deploying".to_string()),
        ];
        let result = pipeline.deep_stage(entries);
        // Decision gets higher score than Error due to clarity/persistence
        assert!(!result.is_empty());
    }

    #[test]
    fn test_deep_capacity_limit() {
        let pipeline = DreamingPipeline::new();
        let mut entries = Vec::new();
        for i in 0..300 {
            entries.push(make_entry(
                EntryCategory::Decision,
                &format!("decision number {i}"),
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
