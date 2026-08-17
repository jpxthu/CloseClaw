//! Memory Miner — two-stage LLM extraction from session transcripts.
//!
//! Miner 1 extracts structured events (title, summary, body, category)
//! from a cleaned transcript via LLM. Miner 2 assigns entities to each
//! event from the entity/type catalog. Results are written to SQLite.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use thiserror::Error;

use closeclaw_config::agents::{
    default_forgetting_initial_ttl_days, default_forgetting_reidentify_extension_days,
    default_mining_dedup_window_days, default_mining_max_events_per_session, MemoryConfig,
    MiningConfig,
};
use closeclaw_session::persistence::{PersistenceError, PersistenceService};

use crate::embedding::{cosine_similarity, EntityEmbedder, NgramEmbedder};
use crate::miner_llm::{MinerLlmCaller, MinerLlmError};
use crate::miner_transcript::clean_transcript;

mod miner_sqlite;
pub(crate) use miner_sqlite::*;

/// Errors specific to the memory-miner.
#[derive(Debug, Error)]
pub enum MinerError {
    /// Storage layer error.
    #[error("storage error: {0}")]
    Storage(#[from] PersistenceError),

    /// An I/O error occurred while reading or writing memory files.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The transcript could not be parsed.
    #[error("transcript parse error: {0}")]
    TranscriptParse(String),

    /// LLM extraction or assignment failed.
    #[error("llm error: {0}")]
    Llm(#[from] MinerLlmError),

    /// SQLite error.
    #[error("sqlite error: {0}")]
    Sqlite(String),

    /// Entity name exceeds the 10-word limit.
    #[error("entity name too long (max 10 words): {0}")]
    EntityNameTooLong(String),
}

/// Category of a mining event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiningEventCategory {
    /// Agent made a clear error.
    Error,
    /// Owner expressed dissatisfaction or correction.
    Anger,
    /// Owner made an explicit product decision.
    Decision,
    /// Agent discovered a simple pattern through repeated attempts,
    /// worth crystallising as experience.
    Insight,
}

impl std::fmt::Display for MiningEventCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Anger => write!(f, "anger"),
            Self::Decision => write!(f, "decision"),
            Self::Insight => write!(f, "insight"),
        }
    }
}

/// A structured event extracted by Miner 1.
#[derive(Debug, Clone)]
pub struct MiningEvent {
    /// Short title for the event.
    pub title: String,
    /// Brief summary of the event.
    pub summary: String,
    /// Full body text of the event.
    pub body: String,
    /// Event category.
    pub category: MiningEventCategory,
    /// Actionable lesson (required for Error/Anger, optional for Decision).
    pub lesson: Option<String>,
    /// If set, the LLM identified this event as a re-occurrence of an
    /// existing event. `write_to_sqlite` will extend `expires_at` on the
    /// referenced event instead of inserting a new row.
    pub reidentified_event_id: Option<i64>,
}

/// An entity assigned to an event by Miner 2.
#[derive(Debug, Clone)]
pub struct MiningEntity {
    /// Entity type (from 11 entity types).
    pub entity_type: String,
    /// Human-readable entity name (max 10 words).
    pub name: String,
    /// Brief entity description.
    pub description: String,
    /// If set, the entity already exists in SQLite with this id.
    /// Used by name-first matching to avoid duplicate inserts.
    pub existing_id: Option<i64>,
}

/// Result of a single mining operation.
#[derive(Debug)]
pub struct MineResult {
    /// Events extracted from the session.
    pub events: Vec<MiningEvent>,
    /// Entity names associated with each event.
    pub entity_names: Vec<Vec<String>>,
}

/// Configuration for the memory miner.
#[derive(Debug, Clone)]
pub struct MinerConfig {
    /// Whether mining is enabled.
    pub enabled: bool,
    /// Model for Miner 1 and Miner 2. `None` means inherit global default.
    pub model: Option<String>,
    /// Maximum events per session.
    pub max_events_per_session: usize,
    /// Dedup window in days for recent event lookup.
    pub dedup_window_days: i32,
    /// Transcript clean rules.
    pub clean_rules: closeclaw_config::agents::TranscriptCleanRules,
    /// Initial TTL in days for new event `expires_at`. Default 90.
    pub initial_ttl_days: i64,
    /// Days to extend `expires_at` when Miner 1 dedup re-identifies an entity.
    /// Default 90.
    pub reidentify_extension_days: i64,
}

impl MinerConfig {
    /// Create a config from a [`MiningConfig`].
    ///
    /// Uses the default `initial_ttl_days` (90). Prefer [`Self::from_memory_config`]
    /// when the full [`MemoryConfig`] is available.
    pub fn from_mining_config(config: &MiningConfig) -> Self {
        Self {
            enabled: config.enabled.unwrap_or(false),
            model: config.model.clone(),
            max_events_per_session: config
                .max_events_per_session
                .unwrap_or_else(default_mining_max_events_per_session)
                as usize,
            dedup_window_days: config
                .dedup_window_days
                .unwrap_or_else(default_mining_dedup_window_days),
            clean_rules: config.transcript_clean_rules.clone(),
            initial_ttl_days: default_forgetting_initial_ttl_days(),
            reidentify_extension_days: default_forgetting_reidentify_extension_days(),
        }
    }

    /// Create a config from a full [`MemoryConfig`].
    ///
    /// Reads both `mining` and `forgetting` sections. This is the preferred
    /// constructor when the caller has access to the complete memory config.
    pub fn from_memory_config(config: &MemoryConfig) -> Self {
        Self {
            enabled: config.mining.enabled.unwrap_or(false),
            model: config.mining.model.clone(),
            max_events_per_session: config
                .mining
                .max_events_per_session
                .unwrap_or_else(default_mining_max_events_per_session)
                as usize,
            dedup_window_days: config
                .mining
                .dedup_window_days
                .unwrap_or_else(default_mining_dedup_window_days),
            clean_rules: config.mining.transcript_clean_rules.clone(),
            initial_ttl_days: config
                .forgetting
                .initial_ttl_days
                .unwrap_or_else(default_forgetting_initial_ttl_days),
            reidentify_extension_days: config
                .forgetting
                .reidentify_extension_days
                .unwrap_or_else(default_forgetting_reidentify_extension_days),
        }
    }
}

impl Default for MinerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: None,
            max_events_per_session: 10,
            dedup_window_days: 30,
            clean_rules: Default::default(),
            initial_ttl_days: default_forgetting_initial_ttl_days(),
            reidentify_extension_days: default_forgetting_reidentify_extension_days(),
        }
    }
}

/// Data loaded from SQLite in a blocking context.
///
/// Used to pass read results from `spawn_blocking` closures to async
/// code without holding a `rusqlite::Connection` across `.await` points.
struct DbReadData {
    /// Recent events text for Miner 1 dedup context.
    recent_events_text: String,
    /// Current MEMORY.md content for Miner 1 dedup context.
    memory_md: String,
    /// Entity/type catalog text for Miner 2.
    catalog: String,
    /// Entity type → similarity_threshold mapping.
    type_thresholds: HashMap<String, f64>,
    /// Existing entities grouped by type for vector fallback matching.
    existing_entities_by_type: HashMap<String, Vec<(i64, String, String, String)>>,
}

/// Memory miner — extracts structured entries from session transcripts.
pub struct MemoryMiner {
    /// Mining configuration.
    config: Arc<RwLock<MinerConfig>>,
    /// LLM caller for Miner 1 (event extraction).
    miner1_llm: Box<dyn MinerLlmCaller>,
    /// LLM caller for Miner 2 (entity assignment).
    miner2_llm: Box<dyn MinerLlmCaller>,
    /// Path to the SQLite database.
    db_path: PathBuf,
    /// Path to MEMORY.md for dedup.
    memory_md_path: String,
}

impl MemoryMiner {
    /// Create a new miner with the given dependencies.
    pub fn new(
        config: MinerConfig,
        miner1_llm: Box<dyn MinerLlmCaller>,
        miner2_llm: Box<dyn MinerLlmCaller>,
        db_path: impl AsRef<Path>,
        memory_md_path: impl Into<String>,
    ) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            miner1_llm,
            miner2_llm,
            db_path: db_path.as_ref().to_path_buf(),
            memory_md_path: memory_md_path.into(),
        }
    }

    /// Returns `true` if mining is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.read().unwrap().enabled
    }

    /// Returns the configured LLM model, or `None` to inherit the global default.
    pub fn model(&self) -> Option<String> {
        self.config.read().unwrap().model.clone()
    }

    /// Update the miner configuration at runtime.
    pub fn update_config(&self, config: MinerConfig) {
        *self.config.write().unwrap() = config;
    }

    /// Mine a single session: clean transcript → extract → assign → write → mark.
    ///
    /// `raw_transcript` is the raw session transcript text.
    pub async fn mine_session(
        &self,
        session_id: &str,
        raw_transcript: &str,
        agent_id: &str,
        storage: &dyn PersistenceService,
    ) -> Result<MineResult, MinerError> {
        if !self.config.read().unwrap().enabled {
            return Ok(MineResult {
                events: Vec::new(),
                entity_names: Vec::new(),
            });
        }

        let checkpoint = storage.load_checkpoint(session_id).await?.ok_or_else(|| {
            MinerError::TranscriptParse(format!("session {session_id} not found"))
        })?;

        if checkpoint.mined {
            return Ok(MineResult {
                events: Vec::new(),
                entity_names: Vec::new(),
            });
        }

        self.mine_session_inner(session_id, raw_transcript, agent_id, &checkpoint, storage)
            .await
    }

    /// Mine a session from a pre-loaded checkpoint.
    ///
    /// Same as [`mine_session`] but accepts the checkpoint directly,
    /// avoiding a redundant storage load. The caller is responsible for
    /// verifying that the session is archived and unmined.
    pub async fn mine_session_from_checkpoint(
        &self,
        session_id: &str,
        raw_transcript: &str,
        agent_id: &str,
        checkpoint: &closeclaw_session::persistence::SessionCheckpoint,
        storage: &dyn PersistenceService,
    ) -> Result<MineResult, MinerError> {
        if !self.config.read().unwrap().enabled {
            return Ok(MineResult {
                events: Vec::new(),
                entity_names: Vec::new(),
            });
        }

        if checkpoint.mined {
            return Ok(MineResult {
                events: Vec::new(),
                entity_names: Vec::new(),
            });
        }

        self.mine_session_inner(session_id, raw_transcript, agent_id, checkpoint, storage)
            .await
    }

    /// Shared mining implementation.
    ///
    /// Separates blocking SQLite operations from async LLM calls to
    /// ensure the `rusqlite::Connection` (which is not `Send`) is dropped
    /// before any `.await` point.
    /// Read config, clean transcript, return (cleaned, dedup_days).
    fn prepare_transcript(&self, raw_transcript: &str) -> Result<(String, i32), MinerError> {
        let cfg = self.config.read().unwrap();
        let cleaned = clean_transcript(raw_transcript, &cfg.clean_rules);
        Ok((cleaned, cfg.dedup_window_days))
    }

    async fn mine_session_inner(
        &self,
        session_id: &str,
        raw_transcript: &str,
        agent_id: &str,
        _checkpoint: &closeclaw_session::persistence::SessionCheckpoint,
        storage: &dyn PersistenceService,
    ) -> Result<MineResult, MinerError> {
        let (cleaned, dedup_days) = self.prepare_transcript(raw_transcript)?;
        if cleaned.is_empty() {
            return Ok(MineResult {
                events: Vec::new(),
                entity_names: Vec::new(),
            });
        }
        let db_data = read_db_data(
            &self.db_path,
            session_id,
            agent_id,
            dedup_days,
            &self.memory_md_path,
        )
        .await?;
        let events = self
            .extract_events(&cleaned, &db_data.recent_events_text, &db_data.memory_md)
            .await?;
        let mut entities = self.miner2_llm.assign_entities(&events, &db_data.catalog).await?;
        for e in &mut entities {
            truncate_entity_names(e);
        }
        // Phase 3.2: Name-first matching (reuse existing entities).
        // This runs before similarity filtering — name priority > vector fallback.
        match_entities_by_name(&self.db_path, agent_id, &mut entities)?;
        filter_entities_by_similarity(
            &mut entities,
            &db_data.type_thresholds,
            &db_data.existing_entities_by_type,
        );
        let write_cfg = {
            let cfg = self.config.read().unwrap();
            WriteConfig {
                session_id,
                agent_id,
                events: &events,
                entities: &entities,
                initial_ttl_days: cfg.initial_ttl_days,
                reidentify_extension_days: cfg.reidentify_extension_days,
            }
        };
        write_mining_results(&self.db_path, &write_cfg).await?;
        storage.mark_mined(session_id).await?;
        let entity_names: Vec<Vec<String>> = entities
            .iter()
            .map(|es| es.iter().map(|e| e.name.clone()).collect())
            .collect();
        Ok(MineResult {
            events,
            entity_names,
        })
    }

    /// Miner 1: extract events from cleaned transcript via LLM.
    async fn extract_events(
        &self,
        cleaned: &str,
        existing_events: &str,
        existing_memory: &str,
    ) -> Result<Vec<MiningEvent>, MinerError> {
        let mut events = self
            .miner1_llm
            .extract_events(cleaned, existing_events, existing_memory)
            .await?;
        let max = self.config.read().unwrap().max_events_per_session;
        events.truncate(max);
        Ok(events)
    }

    /// Run periodic forgetting cleanup: delete expired events and orphan entities.
    ///
    /// Uses `spawn_blocking` to keep the SQLite connection off the async
    /// runtime, matching the pattern of `mine_session_inner`.
    pub async fn run_forgetting_cleanup(
        &self,
    ) -> Result<crate::forgetting::ForgettingCleanupStats, MinerError> {
        let db_path = self.db_path.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = rusqlite::Connection::open(&db_path)
                .map_err(|e| MinerError::Sqlite(e.to_string()))?;
            crate::miner::init_schema(&conn)?;
            let now = chrono::Utc::now().timestamp();
            crate::forgetting::cleanup_expired(&mut conn, now)
        })
        .await
        .map_err(|e| MinerError::Sqlite(e.to_string()))?
    }
}

// ── Extracted mining phases ───────────────────────────────────────────

/// Phase 1: Blocking SQLite reads (spawn_blocking).
///
/// Opens a temporary connection, initialises the schema, and loads
/// recent events, MEMORY.md, the entity catalog, and type thresholds.
/// The connection is dropped before any `.await`.
async fn read_db_data(
    db_path: &Path,
    session_id: &str,
    agent_id: &str,
    dedup_days: i32,
    memory_md_path: &str,
) -> Result<DbReadData, MinerError> {
    let db_path = db_path.to_path_buf();
    let session_id = session_id.to_string();
    let agent_id = agent_id.to_string();
    let memory_md_path = memory_md_path.to_string();
    tokio::task::spawn_blocking(move || -> Result<DbReadData, MinerError> {
        let conn =
            rusqlite::Connection::open(&db_path).map_err(|e| MinerError::Sqlite(e.to_string()))?;
        init_schema(&conn)?;
        let (recent_events_text, _recent_event_ids) =
            load_recent_events(&conn, &session_id, &agent_id, dedup_days)?;
        let memory_md = std::fs::read_to_string(&memory_md_path).unwrap_or_default();
        let catalog = load_entity_catalog(&conn, &agent_id)?;
        let type_thresholds = load_entity_type_thresholds(&conn)?;
        let existing_entities_by_type =
            load_existing_entities_by_type(&conn, &agent_id)?;
        Ok(DbReadData {
            recent_events_text,
            memory_md,
            catalog,
            type_thresholds,
            existing_entities_by_type,
        })
    })
    .await
    .map_err(|e| MinerError::Sqlite(e.to_string()))?
}

/// Phase 3.2: Name-first entity matching.
///
/// For each LLM-produced entity, checks SQLite for an existing entity
/// with the same agent_id + type + normalized_name. If found, replaces
/// the LLM entity's description with the existing one and sets
/// `existing_id` so the write phase reuses the row instead of inserting.
pub(crate) fn match_entities_by_name(
    db_path: &Path,
    agent_id: &str,
    entities: &mut [Vec<MiningEntity>],
) -> Result<(), MinerError> {
    let db_path = db_path.to_path_buf();
    let agent_id = agent_id.to_string();
    let existing_map = load_existing_entities_for_agent(&db_path, &agent_id)?;
    for event_entities in entities.iter_mut() {
        for entity in event_entities.iter_mut() {
            let norm = normalize_entity_name(&entity.name);
            if let Some(&(existing_id, ref existing_desc)) =
                existing_map.get(&(entity.entity_type.clone(), norm))
            {
                entity.existing_id = Some(existing_id);
                entity.description = existing_desc.clone();
            }
        }
    }
    Ok(())
}

/// Phase 3.5: Vector fallback entity matching.
///
/// For each candidate entity not matched by name (i.e. `existing_id` is
/// `None`), computes its vector similarity to existing entities of the
/// same type already stored in SQLite. If the highest similarity score
/// meets or exceeds the per-type `similarity_threshold`, the candidate
/// is replaced by the existing entity (reusing its name and description).
/// Otherwise the candidate is kept as a new entity.
///
/// Candidates already matched by name (`existing_id` is `Some`) are
/// never modified.
fn filter_entities_by_similarity(
    entities: &mut [Vec<MiningEntity>],
    type_thresholds: &HashMap<String, f64>,
    existing_entities_by_type: &HashMap<String, Vec<(i64, String, String, String)>>,
) {
    // Build a corpus from all candidate entities and all existing entities
    // so the NgramEmbedder vocabulary covers every text we need to embed.
    let mut corpus: Vec<String> = Vec::new();
    for event_entities in entities.iter() {
        for entity in event_entities {
            corpus.push(format!("{} {}", entity.name, entity.description));
        }
    }
    for existing_list in existing_entities_by_type.values() {
        for (_id, name, desc, _norm) in existing_list {
            corpus.push(format!("{name} {desc}"));
        }
    }
    if corpus.is_empty() {
        return;
    }
    let corpus_refs: Vec<&str> = corpus.iter().map(|s| s.as_str()).collect();
    let embedder = NgramEmbedder::new(&corpus_refs);

    for event_entities in entities.iter_mut() {
        for candidate in event_entities.iter_mut() {
            // Skip entities already matched by name (Phase 3.2).
            if candidate.existing_id.is_some() {
                continue;
            }
            let threshold = type_thresholds
                .get(&candidate.entity_type)
                .copied()
                .unwrap_or(0.80);
            let existing_list = match existing_entities_by_type.get(&candidate.entity_type) {
                Some(list) => list,
                None => continue,
            };
            if existing_list.is_empty() {
                continue;
            }
            let candidate_text = format!("{} {}", candidate.name, candidate.description);
            let candidate_emb = embedder.embed(&candidate_text);
            // Find the existing entity with the highest similarity.
            let best = existing_list
                .iter()
                .map(|(id, name, desc, _norm)| {
                    let text = format!("{name} {desc}");
                    let emb = embedder.embed(&text);
                    let sim = cosine_similarity(&candidate_emb, &emb);
                    (sim, id, name, desc)
                })
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((sim, id, name, desc)) = best {
                if sim >= threshold {
                    // Reuse the existing entity — adopt its name and description.
                    candidate.existing_id = Some(*id);
                    candidate.name = name.clone();
                    candidate.description = desc.clone();
                }
            }
        }
    }
}

/// Phase 4: Blocking SQLite writes (spawn_blocking).
async fn write_mining_results(
    db_path: &Path,
    write_cfg: &WriteConfig<'_>,
) -> Result<(), MinerError> {
    let db_path = db_path.to_path_buf();
    let session_id = write_cfg.session_id.to_string();
    let agent_id = write_cfg.agent_id.to_string();
    let events: Vec<MiningEvent> = write_cfg.events.to_vec();
    let entities: Vec<Vec<MiningEntity>> = write_cfg.entities.to_vec();
    let initial_ttl_days = write_cfg.initial_ttl_days;
    let reidentify_extension_days = write_cfg.reidentify_extension_days;
    tokio::task::spawn_blocking(move || {
        let conn =
            rusqlite::Connection::open(&db_path).map_err(|e| MinerError::Sqlite(e.to_string()))?;
        write_to_sqlite(
            &conn,
            &WriteConfig {
                session_id: &session_id,
                agent_id: &agent_id,
                events: &events,
                entities: &entities,
                initial_ttl_days,
                reidentify_extension_days,
            },
        )
    })
    .await
    .map_err(|e| MinerError::Sqlite(e.to_string()))?
}

/// Truncate entity names to 10 words maximum.
pub(crate) fn truncate_entity_names(entities: &mut [MiningEntity]) {
    for entity in entities.iter_mut() {
        let words: Vec<&str> = entity.name.split_whitespace().collect();
        if words.len() > 10 {
            entity.name = words[..10].join(" ");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rusqlite::params;

    #[test]
    fn test_miner_config_default_enabled_is_false() {
        let config = MinerConfig::default();
        assert!(
            !config.enabled,
            "MinerConfig::default().enabled should be false per config.md mining.enabled default"
        );
    }

    #[test]
    fn test_normalize_entity_name() {
        assert_eq!(normalize_entity_name("My Entity"), "my_entity");
        assert_eq!(normalize_entity_name("UPPER CASE"), "upper_case");
        assert_eq!(normalize_entity_name("single"), "single");
    }

    #[test]
    fn test_truncate_entity_names() {
        let mut entities = vec![MiningEntity {
            entity_type: "subject".to_string(),
            name: "one two three four five six seven eight nine ten eleven".to_string(),
            description: "".to_string(),
            existing_id: None,
        }];
        truncate_entity_names(&mut entities);
        assert_eq!(
            entities[0].name,
            "one two three four five six seven eight nine ten"
        );
    }

    #[test]
    fn test_truncate_entity_names_within_limit() {
        let mut entities = vec![MiningEntity {
            entity_type: "subject".to_string(),
            name: "short name".to_string(),
            description: "".to_string(),
            existing_id: None,
        }];
        truncate_entity_names(&mut entities);
        assert_eq!(entities[0].name, "short name");
    }

    #[test]
    fn test_load_recent_events_empty_db() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let (result, ids) = load_recent_events(&conn, "other", "agent-1", 30).unwrap();
        assert!(result.is_empty());
        assert!(ids.is_empty());
    }

    #[test]
    fn test_load_recent_events_with_data() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let ts = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO events (title, summary, content,
             category, lesson, source_session_id, agent_id, timestamp, updated_at)
             VALUES ('title', 'summary', 'body',
             'error', 'lesson', 'other-sess', 'agent-1', ?1, ?1)",
            params![ts],
        )
        .unwrap();
        let (result, ids) = load_recent_events(&conn, "my-sess", "agent-1", 30).unwrap();
        assert!(result.contains("title"));
        assert!(result.contains("[error]"));
        assert_eq!(ids.len(), 1);
        assert!(ids[0] > 0);
    }

    #[test]
    fn test_load_recent_events_excludes_old() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let old_ts = Utc::now().timestamp() - (60 * 86400);
        conn.execute(
            "INSERT INTO events (title, summary, content,
             category, lesson, source_session_id, agent_id, timestamp, updated_at)
             VALUES ('old', 'old', 'body',
             'decision', NULL, 'other', 'agent-1', ?1, ?1)",
            params![old_ts],
        )
        .unwrap();
        let (result, ids) = load_recent_events(&conn, "my-sess", "agent-1", 30).unwrap();
        assert!(result.is_empty());
        assert!(ids.is_empty());
    }

    #[test]
    fn test_load_recent_events_cross_agent_isolation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let ts = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO events (title, summary, content,
             category, lesson, source_session_id, agent_id, timestamp, updated_at)
             VALUES ('a1-event', 'a1-summary', 'body',
             'error', 'lesson', 'sess-a', 'agent-1', ?1, ?1)",
            params![ts],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (title, summary, content,
             category, lesson, source_session_id, agent_id, timestamp, updated_at)
             VALUES ('a2-event', 'a2-summary', 'body',
             'error', 'lesson', 'sess-b', 'agent-2', ?1, ?1)",
            params![ts],
        )
        .unwrap();
        let (result_a1, ids_a1) = load_recent_events(&conn, "other", "agent-1", 30).unwrap();
        assert!(result_a1.contains("a1-event"));
        assert!(!result_a1.contains("a2-event"));
        assert_eq!(ids_a1.len(), 1);
        let (result_a2, ids_a2) = load_recent_events(&conn, "other", "agent-2", 30).unwrap();
        assert!(result_a2.contains("a2-event"));
        assert!(!result_a2.contains("a1-event"));
        assert_eq!(ids_a2.len(), 1);
    }

    #[test]
    fn test_load_entity_catalog_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let result = load_entity_catalog(&conn, "agent-1").unwrap();
        // No entities, but all 11 type definitions should be present.
        assert!(result.contains("## subject (主题):"));
        assert!(result.contains("## action (动作):"));
        assert!(result.contains("## tags (标签):"));
        let sections: Vec<&str> = result.split("\n\n").collect();
        assert_eq!(sections.len(), 11, "should contain all 11 type definitions");
    }

    #[test]
    fn test_load_entity_catalog_sorted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO entities (agent_id, type, name, normalized_name, description)
             VALUES ('a1', 'subject', 'Banana', 'banana', 'desc1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO entities (agent_id, type, name, normalized_name, description)
             VALUES ('a1', 'action', 'Apple', 'apple', 'desc2')",
            [],
        )
        .unwrap();
        let result = load_entity_catalog(&conn, "a1").unwrap();
        // Types are sorted alphabetically: action before subject.
        let action_pos = result.find("## action (动作):").unwrap();
        let subject_pos = result.find("## subject (主题):").unwrap();
        assert!(
            action_pos < subject_pos,
            "action should come before subject"
        );
        // Entities appear under their type headers.
        assert!(result.contains("- Apple: desc2"));
        assert!(result.contains("- Banana: desc1"));
    }

    #[test]
    fn test_init_schema_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
    }
}
