//! Memory Miner — two-stage LLM extraction from session transcripts.
//!
//! Miner 1 extracts structured events (title, summary, body, category)
//! from a cleaned transcript via LLM. Miner 2 assigns entities to each
//! event from the entity/type catalog. Results are written to SQLite.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use chrono::Utc;
use rusqlite::params;
use thiserror::Error;

use closeclaw_config::agents::{
    default_mining_dedup_window_days, default_mining_max_events_per_session, MiningConfig,
};
use closeclaw_session::persistence::{PersistenceError, PersistenceService};

use crate::miner_llm::{MinerLlmCaller, MinerLlmError};
use crate::miner_transcript::clean_transcript;

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
}

impl std::fmt::Display for MiningEventCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Anger => write!(f, "anger"),
            Self::Decision => write!(f, "decision"),
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
}

impl MinerConfig {
    /// Create a config from a [`MiningConfig`].
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
        }
    }
}

/// Data loaded from SQLite in a blocking context.
///
/// Used to pass read results from `spawn_blocking` closures to async
/// code without holding a `rusqlite::Connection` across `.await` points.
struct DbReadData {
    /// Recent events text for Miner 1 dedup context.
    recent_events: String,
    /// Current MEMORY.md content for Miner 1 dedup context.
    memory_md: String,
    /// Entity/type catalog text for Miner 2.
    catalog: String,
}

/// Memory miner — extracts structured entries from session transcripts.
pub struct MemoryMiner {
    /// Mining configuration.
    config: Arc<RwLock<MinerConfig>>,
    /// LLM caller for extraction and assignment.
    llm: Box<dyn MinerLlmCaller>,
    /// Path to the SQLite database.
    db_path: PathBuf,
    /// Path to MEMORY.md for dedup.
    memory_md_path: String,
    /// Agent ID for entity scoping.
    agent_id: String,
}

impl MemoryMiner {
    /// Create a new miner with the given dependencies.
    pub fn new(
        config: MinerConfig,
        llm: Box<dyn MinerLlmCaller>,
        db_path: impl AsRef<Path>,
        memory_md_path: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            llm,
            db_path: db_path.as_ref().to_path_buf(),
            memory_md_path: memory_md_path.into(),
            agent_id: agent_id.into(),
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

        self.mine_session_inner(session_id, raw_transcript, &checkpoint, storage)
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

        self.mine_session_inner(session_id, raw_transcript, checkpoint, storage)
            .await
    }

    /// Shared mining implementation.
    ///
    /// Separates blocking SQLite operations from async LLM calls to
    /// ensure the `rusqlite::Connection` (which is not `Send`) is dropped
    /// before any `.await` point.
    async fn mine_session_inner(
        &self,
        session_id: &str,
        raw_transcript: &str,
        _checkpoint: &closeclaw_session::persistence::SessionCheckpoint,
        storage: &dyn PersistenceService,
    ) -> Result<MineResult, MinerError> {
        let (cleaned, dedup_days) = {
            let cfg = self.config.read().unwrap();
            let cleaned = clean_transcript(raw_transcript, &cfg.clean_rules);
            (cleaned, cfg.dedup_window_days)
        };
        if cleaned.is_empty() {
            return Ok(MineResult {
                events: Vec::new(),
                entity_names: Vec::new(),
            });
        }

        // ── Phase 1: Blocking SQLite reads ────────────────────────
        // All Connection usage is confined to this closure; the
        // connection is dropped before we hit any `.await`.
        let db_path = self.db_path.clone();
        let agent_id = self.agent_id.clone();
        let memory_md_path = self.memory_md_path.clone();
        let session_id_owned = session_id.to_string();

        let db_data = tokio::task::spawn_blocking(move || -> Result<DbReadData, MinerError> {
            let conn = rusqlite::Connection::open(&db_path)
                .map_err(|e| MinerError::Sqlite(e.to_string()))?;
            init_schema(&conn)?;

            let recent_events = load_recent_events(&conn, &session_id_owned, dedup_days)?;
            let memory_md = std::fs::read_to_string(&memory_md_path).unwrap_or_default();
            let catalog = load_entity_catalog(&conn, &agent_id)?;

            Ok(DbReadData {
                recent_events,
                memory_md,
                catalog,
            })
        })
        .await
        .map_err(|e| MinerError::Sqlite(e.to_string()))??;

        // ── Phase 2: Async LLM extraction (no Connection in scope) ─
        let events = self
            .extract_events(&cleaned, &db_data.recent_events, &db_data.memory_md)
            .await?;

        // ── Phase 3: Async LLM entity assignment ──────────────────
        let mut entities = self.llm.assign_entities(&events, &db_data.catalog).await?;
        for event_entities in &mut entities {
            truncate_entity_names(event_entities);
        }

        // ── Phase 4: Blocking SQLite writes ───────────────────────
        let db_path = self.db_path.clone();
        let agent_id = self.agent_id.clone();
        let session_id_owned = session_id.to_string();
        let events_clone = events.clone();
        let entities_clone = entities.clone();
        tokio::task::spawn_blocking(move || -> Result<(), MinerError> {
            let conn = rusqlite::Connection::open(&db_path)
                .map_err(|e| MinerError::Sqlite(e.to_string()))?;
            write_to_sqlite(
                &conn,
                &session_id_owned,
                &agent_id,
                &events_clone,
                &entities_clone,
            )
        })
        .await
        .map_err(|e| MinerError::Sqlite(e.to_string()))??;

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
            .llm
            .extract_events(cleaned, existing_events, existing_memory)
            .await?;
        let max = self.config.read().unwrap().max_events_per_session;
        events.truncate(max);
        Ok(events)
    }
}

// ── SQLite operations ─────────────────────────────────────────────────

/// Initialize the SQLite schema for mining tables.
///
/// Creates the `events`, `entities`, `event_entities`, and `entity_types`
/// tables. The `entity_types` table is seeded with the 11 SAG entity types
/// (INSERT OR IGNORE ensures idempotency).
pub(crate) fn init_schema(conn: &rusqlite::Connection) -> Result<(), MinerError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            content TEXT NOT NULL,
            category TEXT NOT NULL,
            lesson TEXT,
            source_session_id TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS entities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            type TEXT NOT NULL,
            name TEXT NOT NULL,
            normalized_name TEXT NOT NULL,
            description TEXT DEFAULT '',
            UNIQUE(agent_id, type, normalized_name)
        );
        CREATE TABLE IF NOT EXISTS event_entities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id INTEGER NOT NULL,
            entity_id INTEGER NOT NULL,
            FOREIGN KEY (event_id) REFERENCES events(id),
            FOREIGN KEY (entity_id) REFERENCES entities(id)
        );
        CREATE TABLE IF NOT EXISTS entity_types (
            id INTEGER PRIMARY KEY,
            type TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            weight REAL NOT NULL DEFAULT 1.0,
            similarity_threshold REAL NOT NULL DEFAULT 0.80,
            is_default INTEGER NOT NULL DEFAULT 0,
            is_active INTEGER NOT NULL DEFAULT 1
        );
        INSERT OR IGNORE INTO entity_types (id, type, name, description, weight, similarity_threshold, is_default, is_active) VALUES
            (1,  'time',         '时间',     '时间点、时期、日期、年份等时间表达', 1.0, 0.90, 0, 1),
            (2,  'location',      '地点',     '国家、城市、地区、地点等物理位置', 1.0, 0.75, 0, 1),
            (3,  'person',        '人物',     '人物和具名个体（含 agent 角色、用户身份）', 1.2, 0.80, 0, 1),
            (4,  'organization',  '组织',     '公司、机构、团队等组织', 1.1, 0.80, 0, 1),
            (5,  'subject',       '主题',     '主要主题、概念和课题', 1.5, 0.78, 1, 1),
            (6,  'product',       '产品',     '产品、服务、项目和命名交付物', 1.1, 0.80, 0, 1),
            (7,  'metric',        '指标',     '数字、指标、度量、金额和统计数据', 1.2, 0.85, 0, 1),
            (8,  'action',        '动作',     '重要动作、变更、决策和操作', 1.3, 0.78, 1, 1),
            (9,  'work',          '作品',     '创作物、文档、论文、书籍、报告', 1.0, 0.80, 0, 1),
            (10, 'group',         '群体',     '群体、社区、受众和人口', 1.0, 0.78, 0, 1),
            (11, 'tags',          '标签',     '兜底标签，当无特定类型匹配时使用', 0.5, 0.70, 1, 1);",
    )
    .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    Ok(())
}

/// Write events and entities to SQLite.
pub(crate) fn write_to_sqlite(
    conn: &rusqlite::Connection,
    session_id: &str,
    agent_id: &str,
    events: &[MiningEvent],
    entities: &[Vec<MiningEntity>],
) -> Result<(), MinerError> {
    for (event, event_entities) in events.iter().zip(entities.iter()) {
        let ts = Utc::now().timestamp();
        let event_id: i64 = conn
            .query_row(
                "INSERT INTO events (title, summary, content,
                 category, lesson, source_session_id, timestamp, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 RETURNING id",
                params![
                    event.title,
                    event.summary,
                    event.body,
                    event.category.to_string(),
                    event.lesson,
                    session_id,
                    ts,
                ],
                |row| row.get(0),
            )
            .map_err(|e| MinerError::Sqlite(e.to_string()))?;

        for entity in event_entities {
            let norm_name = normalize_entity_name(&entity.name);
            conn.execute(
                "INSERT OR IGNORE INTO entities (agent_id, type, name, normalized_name, description)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![agent_id, entity.entity_type, entity.name, norm_name, entity.description],
            )
            .map_err(|e| MinerError::Sqlite(e.to_string()))?;
            let entity_id: i64 = conn
                .query_row(
                    "SELECT id FROM entities 
                     WHERE agent_id = ?1 
                     AND type = ?2 
                     AND normalized_name = ?3",
                    params![agent_id, entity.entity_type, norm_name],
                    |row| row.get(0),
                )
                .map_err(|e| MinerError::Sqlite(e.to_string()))?;
            conn.execute(
                "INSERT OR IGNORE INTO event_entities (event_id, entity_id) VALUES (?1, ?2)",
                params![event_id, entity_id],
            )
            .map_err(|e| MinerError::Sqlite(e.to_string()))?;
        }
    }
    Ok(())
}

/// Write entries to SQLite (public interface).
pub fn write_entries_to_db(
    conn: &rusqlite::Connection,
    session_id: &str,
    agent_id: &str,
    events: &[MiningEvent],
    entities: &[Vec<MiningEntity>],
) -> Result<(), MinerError> {
    init_schema(conn)?;
    write_to_sqlite(conn, session_id, agent_id, events, entities)
}

/// Load recent events within the dedup window for Miner 1 context.
pub(crate) fn load_recent_events(
    conn: &rusqlite::Connection,
    exclude_session: &str,
    dedup_window_days: i32,
) -> Result<String, MinerError> {
    let cutoff = Utc::now().timestamp() - (dedup_window_days as i64 * 86400);
    let sql = "SELECT title, summary, category FROM events
               WHERE source_session_id != ?1 AND timestamp >= ?2
               ORDER BY timestamp DESC LIMIT 20";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map(params![exclude_session, cutoff], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| MinerError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows
        .iter()
        .map(|(title, summary, category)| format!("- [{category}] {title}: {summary}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Load entity/type catalog from SQLite, sorted by type → normalized_name.
pub(crate) fn load_entity_catalog(
    conn: &rusqlite::Connection,
    agent_id: &str,
) -> Result<String, MinerError> {
    let sql = "SELECT type, name, description FROM entities
               WHERE agent_id = ?1
               ORDER BY type, normalized_name";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map(params![agent_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| MinerError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows
        .iter()
        .map(|(typ, name, desc)| format!("- [{typ}] {name}: {desc}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Normalize an entity name: lowercase, replace spaces with underscores.
pub(crate) fn normalize_entity_name(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
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
        }];
        truncate_entity_names(&mut entities);
        assert_eq!(entities[0].name, "short name");
    }

    #[test]
    fn test_load_recent_events_empty_db() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let result = load_recent_events(&conn, "other", 30).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_load_recent_events_with_data() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let ts = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO events (title, summary, content,
             category, lesson, source_session_id, timestamp, updated_at)
             VALUES ('title', 'summary', 'body',
             'error', 'lesson', 'other-sess', ?1, ?1)",
            params![ts],
        )
        .unwrap();
        let result = load_recent_events(&conn, "my-sess", 30).unwrap();
        assert!(result.contains("title"));
        assert!(result.contains("[error]"));
    }

    #[test]
    fn test_load_recent_events_excludes_old() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let old_ts = Utc::now().timestamp() - (60 * 86400);
        conn.execute(
            "INSERT INTO events (title, summary, content,
             category, lesson, source_session_id, timestamp, updated_at)
             VALUES ('old', 'old', 'body',
             'decision', NULL, 'other', ?1, ?1)",
            params![old_ts],
        )
        .unwrap();
        let result = load_recent_events(&conn, "my-sess", 30).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_load_entity_catalog_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        let result = load_entity_catalog(&conn, "agent-1").unwrap();
        assert!(result.is_empty());
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
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("action"));
        assert!(lines[1].contains("subject"));
    }

    #[test]
    fn test_init_schema_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
    }
}
