//! Dreaming — three-stage memory promotion pipeline.
//!
//! Consumes structured memory entries produced by the memory-miner and
//! promotes high-value ones to MEMORY.md through Light → REM → Deep stages.
//!
//! Light / REM / Deep are programmatic; lesson consolidation is LLM-driven.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rusqlite::params;
use thiserror::Error;

use closeclaw_config::agents::{
    default_capacity_max_rules, default_diary_path, default_memory_md_path,
    default_scoring_cross_agent, default_scoring_entity_type_weight, default_scoring_explicitness,
    default_scoring_frequency, default_scoring_negative_signal, default_scoring_recency,
    default_threshold_absolute, default_threshold_relative, DreamingConfig, DreamingScoringConfig,
};
use closeclaw_session::persistence::{DreamingStatus, PersistenceError, PersistenceService};

use crate::dreaming_llm::{DreamingLlmCaller, DreamingLlmError, PromotedGroupInfo};

// ── Types ────────────────────────────────────────────────────────────────

type EventEntityRow = (
    i64,
    String,
    String,
    Option<String>,
    i64,
    i64,
    String,
    String,
);

/// A structured memory entry produced by the memory-miner.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEntry {
    pub category: EntryCategory,
    pub body: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub source_session_id: String,
    pub lesson: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) score: f64,
    pub event_id: i64,
    pub entity_type: String,
    pub entity_name: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl MemoryEntry {
    /// Set the lesson field (builder pattern).
    pub fn lesson(mut self, lesson: String) -> Self {
        self.lesson = Some(lesson);
        self
    }
}

/// Entity group: entries sharing the same (entity_name, entity_type).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct EntityGroup {
    pub entity_name: String,
    pub entity_type: String,
    pub entries: Vec<MemoryEntry>,
    pub frequency: usize,
    pub cross_agent_count: usize,
    pub score: f64,
}

/// Memory entry category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryCategory {
    Error,
    Anger,
    Decision,
}

/// Errors specific to the dreaming pipeline.
#[derive(Debug, Error)]
pub enum DreamingError {
    #[error("storage error: {0}")]
    Storage(#[from] PersistenceError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("data error: {0}")]
    Data(String),
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("llm error: {0}")]
    Llm(#[from] DreamingLlmError),
}

/// Thresholds for the Deep stage three-gate filtering.
#[derive(Debug, Clone)]
struct Thresholds {
    absolute: f64,
    relative: f64,
    max_rules: usize,
}

/// Orchestrates the dreaming pipeline: Light → REM → Deep → LLM consolidation → MEMORY.md.
pub struct DreamingPipeline {
    scoring: DreamingScoringConfig,
    thresholds: Thresholds,
    config: Arc<RwLock<DreamingConfig>>,
    model: Arc<RwLock<Option<String>>>,
    db_path: Option<PathBuf>,
    memory_md_path: String,
    llm: Option<Arc<dyn DreamingLlmCaller>>,
}

impl DreamingPipeline {
    pub fn model(&self) -> Option<String> {
        self.model.read().unwrap().clone()
    }

    pub fn update_config(&self, config: DreamingConfig) {
        *self.model.write().unwrap() = config.model.clone();
        *self.config.write().unwrap() = config;
    }

    pub fn new() -> Self {
        Self {
            scoring: DreamingScoringConfig::default(),
            thresholds: Thresholds {
                absolute: 2.0,
                relative: 0.3,
                max_rules: 20,
            },
            config: Arc::new(RwLock::new(DreamingConfig::default())),
            model: Arc::new(RwLock::new(None)),
            db_path: None,
            memory_md_path: default_memory_md_path(),
            llm: None,
        }
    }

    pub fn with_config(config: DreamingConfig) -> Self {
        let scoring = config.scoring.clone();
        let model = config.model.clone();
        let thresholds = Thresholds {
            absolute: config
                .threshold
                .absolute
                .unwrap_or_else(default_threshold_absolute),
            relative: config
                .threshold
                .relative
                .unwrap_or_else(default_threshold_relative),
            max_rules: config
                .capacity
                .max_rules
                .unwrap_or_else(default_capacity_max_rules),
        };
        Self {
            scoring,
            thresholds,
            config: Arc::new(RwLock::new(config)),
            model: Arc::new(RwLock::new(model)),
            db_path: None,
            memory_md_path: default_memory_md_path(),
            llm: None,
        }
    }

    pub fn with_db_path(mut self, db_path: impl AsRef<Path>) -> Self {
        self.db_path = Some(db_path.as_ref().to_path_buf());
        self
    }

    pub fn with_memory_md_path(mut self, path: impl Into<String>) -> Self {
        self.memory_md_path = path.into();
        self
    }

    pub fn with_llm(mut self, llm: Arc<dyn DreamingLlmCaller>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Execute one full dreaming cycle.
    pub async fn run_once(&self, storage: &dyn PersistenceService) -> Result<(), DreamingError> {
        {
            let cfg = self.config.read().unwrap();
            if !cfg.enabled.unwrap_or(false) {
                return Ok(());
            }
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
            self.mark_sessions_completed(storage, &session_ids).await?;
            return Ok(());
        }

        let light = self.light_stage(all_entries)?;

        // Transition: Light → REM
        self.mark_sessions_status(storage, &session_ids, DreamingStatus::InRem)
            .await?;

        let entity_groups = self.rem_stage(light);

        // Transition: REM → Deep
        self.mark_sessions_status(storage, &session_ids, DreamingStatus::InDeep)
            .await?;

        let deep = self.deep_stage(entity_groups);

        // LLM lesson consolidation (graceful degradation on failure).
        let rules = if let Some(llm) = &self.llm {
            self.consolidate_lessons(llm, &deep).await
        } else {
            tracing::warn!("no LLM caller configured, skipping lesson consolidation");
            deep.iter()
                .flat_map(|g| {
                    g.entries
                        .iter()
                        .filter_map(|e| e.lesson.clone().or(Some(e.body.clone())))
                })
                .collect()
        };

        // Anti-contamination check and MEMORY.md write.
        let verified = self.verify_and_filter_rules(&rules, &deep).await?;
        if !verified.is_empty() {
            self.write_memory_md(&verified)?;
        }

        // Build promoted groups info for Dream Diary.
        let promoted = self.build_promoted_groups(&deep, &rules, &verified);

        self.mark_sessions_completed(storage, &session_ids).await?;

        // Write Dream Diary if enabled.
        let diary_enabled = self.config.read().unwrap().diary.enabled.unwrap_or(true);
        if diary_enabled && !promoted.is_empty() {
            self.write_dream_diary(&promoted, self.llm.as_deref())
                .await?;
        }

        Ok(())
    }

    /// Batch-update dreaming status for all given sessions.
    pub(crate) async fn mark_sessions_status(
        &self,
        storage: &dyn PersistenceService,
        session_ids: &[String],
        status: DreamingStatus,
    ) -> Result<(), DreamingError> {
        for sid in session_ids {
            storage.update_dreaming_status(sid, status).await?;
        }
        Ok(())
    }

    /// Mark all given sessions as `DreamingStatus::Completed`.
    async fn mark_sessions_completed(
        &self,
        storage: &dyn PersistenceService,
        session_ids: &[String],
    ) -> Result<(), DreamingError> {
        self.mark_sessions_status(storage, session_ids, DreamingStatus::Completed)
            .await
    }

    /// Collect unprocessed entries for a single session from SQLite.
    pub(crate) async fn collect_entries_for_session(
        &self,
        storage: &dyn PersistenceService,
        session_id: &str,
    ) -> Result<Vec<MemoryEntry>, DreamingError> {
        storage
            .update_dreaming_status(session_id, DreamingStatus::InLight)
            .await?;

        let db_path = match &self.db_path {
            Some(p) => p.clone(),
            None => return Ok(Vec::new()),
        };

        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| DreamingError::Sqlite(e.to_string()))?;

        self.load_entries_from_sqlite(&conn, session_id)
    }

    /// Query SQLite for events and entities for a session.
    pub(crate) fn load_entries_from_sqlite(
        &self,
        conn: &rusqlite::Connection,
        session_id: &str,
    ) -> Result<Vec<MemoryEntry>, DreamingError> {
        let sql = "SELECT e.id, e.content, e.category, e.lesson, e.timestamp,
                         e.updated_at, ent.type AS entity_type, ent.name AS entity_name
                    FROM events e
                    JOIN event_entities ee ON ee.event_id = e.id
                    JOIN entities ent ON ent.id = ee.entity_id
                    WHERE e.source_session_id = ?1";

        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()),
        };

        let rows: Vec<EventEntityRow> = stmt
            .query_map(params![session_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            })
            .map_err(|e| DreamingError::Sqlite(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let mut entries = Vec::new();
        for (
            event_id,
            content,
            category_str,
            lesson,
            ts,
            updated_at_ts,
            entity_type,
            entity_name,
        ) in rows
        {
            let category = match category_str.as_str() {
                "error" => EntryCategory::Error,
                "anger" => EntryCategory::Anger,
                "decision" => EntryCategory::Decision,
                _ => continue,
            };

            let timestamp =
                chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(chrono::Utc::now);
            let updated_at =
                chrono::DateTime::from_timestamp(updated_at_ts, 0).unwrap_or_else(chrono::Utc::now);

            // Tags are temporarily filled with entity_name; REM stage will
            // override with keyword aggregation later.
            let tags: Vec<String> = vec![entity_name.clone()];

            entries.push(MemoryEntry {
                category,
                body: content,
                timestamp,
                source_session_id: session_id.to_string(),
                lesson,
                tags,
                score: 0.0,
                event_id,
                entity_type,
                entity_name,
                updated_at,
            });
        }

        Ok(entries)
    }

    // ── Light stage ──────────────────────────────────────────────────

    /// Light stage: deduplicate (batch-internal + MEMORY.md semantic)
    /// and chunk entries by entity type.
    pub(crate) fn light_stage(
        &self,
        entries: Vec<MemoryEntry>,
    ) -> Result<Vec<Vec<MemoryEntry>>, DreamingError> {
        let existing_rules = self.read_existing_rules();
        let deduped = self.deduplicate(entries, &existing_rules);
        Ok(self.chunk_by_entity_type(deduped))
    }

    /// Read existing rules from MEMORY.md for semantic deduplication.
    fn read_existing_rules(&self) -> Vec<String> {
        let path = Path::new(&self.memory_md_path);
        if !path.exists() {
            return Vec::new();
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        content
            .lines()
            .filter_map(|line| line.strip_prefix("- ").map(String::from))
            .collect()
    }

    /// Remove duplicates: batch-internal exact + MEMORY.md semantic dedup.
    pub(crate) fn deduplicate(
        &self,
        entries: Vec<MemoryEntry>,
        existing_rules: &[String],
    ) -> Vec<MemoryEntry> {
        // Layer 1: exact batch-internal dedup.
        let mut seen = std::collections::HashSet::new();
        let exact_deduped: Vec<MemoryEntry> = entries
            .into_iter()
            .filter(|e| {
                let key = (e.category, e.source_session_id.clone(), e.body.clone());
                seen.insert(key)
            })
            .collect();

        // Layer 2: semantic dedup against MEMORY.md existing rules.
        if existing_rules.is_empty() {
            return exact_deduped;
        }

        exact_deduped
            .into_iter()
            .filter(|e| {
                let body_words: Vec<String> = Self::extract_words(&e.body);
                !existing_rules.iter().any(|rule| {
                    let rule_words: Vec<String> = Self::extract_words(rule);
                    Self::word_overlap(&body_words, &rule_words) >= 0.6
                })
            })
            .collect()
    }

    /// Compute Jaccard word overlap between two word sets.
    fn word_overlap(a: &[String], b: &[String]) -> f64 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        let set_a: std::collections::HashSet<&String> = a.iter().collect();
        let set_b: std::collections::HashSet<&String> = b.iter().collect();
        let intersection = set_a.intersection(&set_b).count();
        let union = set_a.len().max(set_b.len());
        intersection as f64 / union as f64
    }

    /// Split entries into groups by entity type.
    pub(crate) fn chunk_by_entity_type(&self, entries: Vec<MemoryEntry>) -> Vec<Vec<MemoryEntry>> {
        let mut groups: std::collections::HashMap<String, Vec<MemoryEntry>> =
            std::collections::HashMap::new();
        for e in entries {
            groups.entry(e.entity_type.clone()).or_default().push(e);
        }
        groups.into_values().collect()
    }

    // ── REM stage ────────────────────────────────────────────────────

    /// Load entity → agent_id mapping from SQLite for cross-agent detection.
    fn load_entity_agent_map(
        &self,
    ) -> std::collections::HashMap<(String, String), std::collections::HashSet<String>> {
        let db_path = match &self.db_path {
            Some(p) => p,
            None => return std::collections::HashMap::new(),
        };
        let conn = match rusqlite::Connection::open(db_path) {
            Ok(c) => c,
            Err(_) => return std::collections::HashMap::new(),
        };
        let mut stmt =
            match conn.prepare("SELECT ent.name, ent.type, ent.agent_id FROM entities ent") {
                Ok(s) => s,
                Err(_) => return std::collections::HashMap::new(),
            };
        let mut map: std::collections::HashMap<
            (String, String),
            std::collections::HashSet<String>,
        > = std::collections::HashMap::new();
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                map.entry((row.0, row.1)).or_default().insert(row.2);
            }
        }
        map
    }

    /// REM stage: cluster entries by entity, compute frequency and cross-agent counts.
    pub(crate) fn rem_stage(&self, chunks: Vec<Vec<MemoryEntry>>) -> Vec<EntityGroup> {
        let all: Vec<MemoryEntry> = chunks.into_iter().flatten().collect();
        let agent_map = self.load_entity_agent_map();
        let mut groups: std::collections::HashMap<(String, String), Vec<MemoryEntry>> =
            std::collections::HashMap::new();
        for e in all {
            groups
                .entry((e.entity_name.clone(), e.entity_type.clone()))
                .or_default()
                .push(e);
        }
        groups
            .into_iter()
            .map(|((name, etype), entries)| {
                let frequency = entries
                    .iter()
                    .map(|e| &e.source_session_id)
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                let cross_agent_count = agent_map
                    .get(&(name.clone(), etype.clone()))
                    .map(std::collections::HashSet::len)
                    .unwrap_or(1);
                EntityGroup {
                    entity_name: name,
                    entity_type: etype,
                    entries,
                    frequency,
                    cross_agent_count,
                    score: 0.0,
                }
            })
            .collect()
    }

    /// Split body text into lowercase words for keyword extraction.
    fn extract_words(body: &str) -> Vec<String> {
        body.split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() > 2)
            .collect()
    }

    // ── Deep stage ───────────────────────────────────────────────────

    /// Deep stage: score entity groups, apply three thresholds, return survivors.
    pub(crate) fn deep_stage(&self, groups: Vec<EntityGroup>) -> Vec<EntityGroup> {
        let mut scored: Vec<EntityGroup> = groups
            .into_iter()
            .map(|g| self.score_entity_group(g))
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Gate 1: absolute threshold
        scored.retain(|g| g.score >= self.thresholds.absolute);

        // Gate 2: relative threshold (per entity_type)
        self.apply_relative_filter(&mut scored);

        // Gate 3: capacity limit (entity group count)
        let existing_count = self.read_existing_rules().len();
        let remaining = self.thresholds.max_rules.saturating_sub(existing_count);
        scored.truncate(remaining.min(scored.len()));

        scored
    }

    /// Compute a weighted score for an entity group across 6 dimensions.
    fn score_entity_group(&self, mut group: EntityGroup) -> EntityGroup {
        let frequency = group.frequency as f64;
        let latest = group
            .entries
            .iter()
            .map(|e| e.timestamp)
            .max()
            .unwrap_or_else(chrono::Utc::now);
        let age_hours = (chrono::Utc::now() - latest).num_hours().max(0) as f64;
        let recency = 1.0 / (1.0 + age_hours / 168.0);
        let decisions = group
            .entries
            .iter()
            .filter(|e| e.category == EntryCategory::Decision)
            .count() as f64;
        let total = group.entries.len().max(1) as f64;
        let explicitness = decisions / total;
        let entity_type_weight = self.load_entity_type_weight(&group.entity_type);
        let cross_agent = group.cross_agent_count as f64;

        // negative_signal: reversal detection — count entries whose
        // category differs from the earliest entry's category.
        let negative_signal = {
            let mut sorted = group.entries.clone();
            sorted.sort_by_key(|e| e.timestamp);
            match sorted.first().map(|e| e.category) {
                Some(fc) => {
                    let reversals = sorted.iter().filter(|e| e.category != fc).count();
                    reversals as f64 / total
                }
                None => 0.0,
            }
        };
        let w = &self.scoring;
        group.score = w.frequency_weight.unwrap_or_else(default_scoring_frequency) * frequency
            + w.recency_weight.unwrap_or_else(default_scoring_recency) * recency
            + w.explicitness_weight
                .unwrap_or_else(default_scoring_explicitness)
                * explicitness
            + w.entity_type_weight_weight
                .unwrap_or_else(default_scoring_entity_type_weight)
                * entity_type_weight
            + w.cross_agent_weight
                .unwrap_or_else(default_scoring_cross_agent)
                * cross_agent
            + w.negative_signal_weight
                .unwrap_or_else(default_scoring_negative_signal)
                * negative_signal;
        group
    }

    /// Query the `entity_types` table for a type's weight.
    fn load_entity_type_weight(&self, entity_type: &str) -> f64 {
        let db_path = match &self.db_path {
            Some(p) => p,
            None => return 1.0,
        };
        let conn = match rusqlite::Connection::open(db_path) {
            Ok(c) => c,
            Err(_) => return 1.0,
        };
        conn.query_row(
            "SELECT weight FROM entity_types WHERE type = ?1",
            params![entity_type],
            |row| row.get::<_, f64>(0),
        )
        .unwrap_or(1.0)
    }

    /// Remove entity groups below `relative × top_score` within same entity_type.
    fn apply_relative_filter(&self, groups: &mut Vec<EntityGroup>) {
        if groups.is_empty() {
            return;
        }

        let types: Vec<String> = groups.iter().map(|g| g.entity_type.clone()).collect();
        let unique_types: std::collections::HashSet<String> = types.into_iter().collect();

        for etype in unique_types {
            let type_scores: Vec<f64> = groups
                .iter()
                .filter(|g| g.entity_type == etype)
                .map(|g| g.score)
                .collect();
            if type_scores.is_empty() {
                continue;
            }
            let max_score = type_scores
                .iter()
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max);
            let cutoff = max_score * self.thresholds.relative;
            groups.retain(|g| g.entity_type != etype || g.score >= cutoff);
        }
    }

    // ── Dream Diary ────────────────────────────────────────────────

    /// Write a Dream Diary summarizing promoted entity groups.
    ///
    /// Uses LLM narrative when available; falls back to structured
    /// summary on LLM failure or absence.
    pub(crate) async fn write_dream_diary(
        &self,
        promoted_groups: &[PromotedGroupInfo],
        llm: Option<&dyn DreamingLlmCaller>,
    ) -> Result<(), DreamingError> {
        let (diary_enabled, diary_path_str) = {
            let cfg = self.config.read().unwrap();
            (
                cfg.diary.enabled.unwrap_or(true),
                cfg.diary.path.clone().unwrap_or_else(default_diary_path),
            )
        };
        if !diary_enabled || promoted_groups.is_empty() {
            return Ok(());
        }

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let filename = format!("{}.md", date);
        let diary_dir = std::path::Path::new(&diary_path_str);
        std::fs::create_dir_all(diary_dir)?;
        let diary_path = diary_dir.join(&filename);

        let total_lessons: usize = promoted_groups.iter().map(|g| g.lessons.len()).sum();
        let mut content = format!("# Dream Diary — {}\n\n", date);
        content.push_str(&format!(
            "Promoted {} lessons ({} groups) to MEMORY.md.\n\n",
            total_lessons,
            promoted_groups.len()
        ));

        // Try LLM narrative; fall back to structured summary.
        let narrative = if let Some(llm) = llm {
            llm.generate_diary_narrative(promoted_groups).await.ok()
        } else {
            None
        };

        if let Some(text) = narrative {
            content.push_str(&text);
        } else {
            // Structured summary fallback.
            for group in promoted_groups {
                for lesson in &group.lessons {
                    content.push_str(&format!(
                        "- **{}** ({}): {}\n",
                        group.entity_name, group.entity_type, lesson
                    ));
                }
            }
        }

        std::fs::write(&diary_path, content)?;
        tracing::info!(path = %diary_path.display(), "dream diary written");
        Ok(())
    }

    // ── LLM lesson consolidation ────────────────────────────────────

    /// Consolidate lessons from entity groups via LLM.
    pub(crate) async fn consolidate_lessons(
        &self,
        llm: &Arc<dyn DreamingLlmCaller>,
        groups: &[EntityGroup],
    ) -> Vec<String> {
        let mut rules = Vec::new();

        for group in groups {
            let lessons: Vec<String> = group
                .entries
                .iter()
                .map(|e| e.lesson.clone().unwrap_or_else(|| e.body.clone()))
                .collect();

            match llm
                .consolidate_lessons(
                    &lessons,
                    &group.entity_name,
                    &group.entity_type,
                    group.frequency,
                )
                .await
            {
                Ok(rule) => rules.push(rule),
                Err(e) => {
                    tracing::warn!(
                        entity = group.entity_name.as_str(),
                        %e,
                        "LLM consolidation failed, using raw lessons"
                    );
                    rules.extend(lessons);
                }
            }
        }
        rules
    }

    // ── Anti-contamination check ─────────────────────────────────────

    /// Build promoted groups info from deep groups, rules, and verified rules.
    ///
    /// Matches each rule to its corresponding group and keeps only those
    /// that passed anti-contamination verification.
    fn build_promoted_groups(
        &self,
        groups: &[EntityGroup],
        rules: &[String],
        verified: &[String],
    ) -> Vec<PromotedGroupInfo> {
        let verified_set: std::collections::HashSet<usize> = rules
            .iter()
            .zip(0..)
            .filter(|(rule, _)| {
                verified
                    .iter()
                    .any(|v| std::ptr::eq(v, *rule) || v == *rule)
            })
            .map(|(_, i)| i)
            .collect();
        groups
            .iter()
            .zip(rules.iter())
            .enumerate()
            .filter(|(i, _)| verified_set.contains(i))
            .map(|(_, (group, rule))| PromotedGroupInfo {
                entity_name: group.entity_name.clone(),
                entity_type: group.entity_type.clone(),
                lessons: vec![rule.clone()],
            })
            .collect()
    }

    /// Verify source events still exist and filter rules accordingly.
    /// Uses event_id + updated_at for anti-contamination check.
    pub(crate) async fn verify_and_filter_rules(
        &self,
        rules: &[String],
        groups: &[EntityGroup],
    ) -> Result<Vec<String>, DreamingError> {
        if self.db_path.is_none() {
            return Ok(rules.to_vec());
        }

        let db_path = self.db_path.as_ref().unwrap();
        let conn = rusqlite::Connection::open(db_path)
            .map_err(|e| DreamingError::Sqlite(e.to_string()))?;

        let mut verified = Vec::new();
        for (rule, group) in rules.iter().zip(groups.iter()) {
            let all_valid = group
                .entries
                .iter()
                .all(|e| self.verify_event_integrity(&conn, e).unwrap_or(false));
            if all_valid {
                verified.push(rule.clone());
            } else {
                tracing::warn!(
                    entity = group.entity_name.as_str(),
                    "source event integrity check failed, skipping rule"
                );
            }
        }
        Ok(verified)
    }

    /// Check event integrity via event_id + updated_at.
    pub(crate) fn verify_event_integrity(
        &self,
        conn: &rusqlite::Connection,
        entry: &MemoryEntry,
    ) -> Result<bool, DreamingError> {
        let sql = "SELECT COUNT(*) FROM events
                    WHERE id = ?1 AND updated_at = ?2";
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| DreamingError::Sqlite(e.to_string()))?;
        let ts = entry.updated_at.timestamp();
        let count: i64 = stmt
            .query_row(params![entry.event_id, ts], |row| row.get(0))
            .map_err(|e| DreamingError::Sqlite(e.to_string()))?;
        Ok(count > 0)
    }

    // ── MEMORY.md write ──────────────────────────────────────────────

    /// Write consolidated rules to MEMORY.md (dedup against existing).
    pub(crate) fn write_memory_md(&self, rules: &[String]) -> Result<(), DreamingError> {
        let path = Path::new(&self.memory_md_path);

        // Read existing rules for deduplication.
        let existing = if path.exists() {
            std::fs::read_to_string(path)?
        } else {
            String::new()
        };
        let existing_rules: std::collections::HashSet<String> = existing
            .lines()
            .filter_map(|line| line.strip_prefix("- ").map(String::from))
            .collect();

        // Append new rules that are not duplicates.
        let mut content = existing;
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        let mut appended = 0;
        for rule in rules {
            if !existing_rules.contains(rule.as_str()) {
                content.push_str(&format!("- {}\n", rule));
                appended += 1;
            }
        }

        if appended > 0 {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, &content)?;
            tracing::info!(
                path = %self.memory_md_path,
                appended,
                "MEMORY.md updated"
            );
        }
        Ok(())
    }
}

impl Default for DreamingPipeline {
    fn default() -> Self {
        Self::new()
    }
}
