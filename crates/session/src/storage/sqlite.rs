//! SQLite storage backend for session persistence
//!
//! This backend stores session checkpoints in a local SQLite database,
//! suitable for single-node deployments without Redis.

mod archive_support;

#[cfg(test)]
mod bug904_tests;
#[cfg(test)]
mod tests;

use crate::persistence::{
    ConsistencyCheckResult, PersistenceError, PersistenceService, SessionCheckpoint,
};
use async_trait::async_trait;
use rusqlite::{params, Connection};
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio::task::spawn_blocking;

/// SQLite storage backend
#[derive(Debug)]
pub struct SqliteStorage {
    data_dir: PathBuf,
}
impl SqliteStorage {
    /// Create a new SqliteStorage instance
    ///
    /// Creates the data directory structure and initializes the SQLite database.
    ///
    /// # Errors
    /// Returns `PersistenceError::Sqlite` if directory creation or DB init fails.
    pub fn new(data_dir: &Path) -> Result<Self, PersistenceError> {
        let data_dir = data_dir.to_path_buf();

        // Create directory structure: sessions/ and archived_sessions/
        let sessions_dir = data_dir.join("sessions");
        let archived_dir = data_dir.join("archived_sessions");
        std::fs::create_dir_all(&sessions_dir)
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        std::fs::create_dir_all(&archived_dir)
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

        // Migrate legacy `sessions.db` to `sessions.sqlite` (per design doc)
        let legacy_db_path = data_dir.join("sessions.db");
        let db_path = data_dir.join("sessions.sqlite");
        if legacy_db_path.exists() {
            std::fs::rename(&legacy_db_path, &db_path)
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        }

        // Open or create SQLite database
        let conn =
            Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

        // Initialize schema
        Self::init_schema(&conn)?;

        Ok(Self { data_dir })
    }
    /// Add a column to sessions table if it doesn't already exist.
    ///
    /// SAFETY: `column` and `col_type` are always hardcoded string literals
    /// passed from `init_schema`, never from user input. This eliminates
    /// SQL injection risk in the format-string `ALTER TABLE` statement.
    fn add_column_if_not_exists(
        conn: &Connection,
        column: &str,
        col_type: &str,
    ) -> Result<(), PersistenceError> {
        let exists = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(sessions)")
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();
            cols.iter().any(|name| name == column)
        };
        if !exists {
            let sql = format!("ALTER TABLE sessions ADD COLUMN {column} {col_type}");
            conn.execute(&sql, [])
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        }
        Ok(())
    }

    /// Initialize the database schema
    fn init_schema(conn: &Connection) -> Result<(), PersistenceError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                role TEXT NOT NULL,
                channel TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                title TEXT,
                last_message_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                archived_at INTEGER,
                message_count INTEGER DEFAULT 0,
                metadata TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_sessions_status
                ON sessions(status);

            CREATE INDEX IF NOT EXISTS idx_sessions_last_message_at
                ON sessions(last_message_at);

            CREATE INDEX IF NOT EXISTS idx_sessions_agent_id
                ON sessions(agent_id);

            CREATE INDEX IF NOT EXISTS idx_sessions_archived_at
                ON sessions(archived_at)
                WHERE archived_at IS NOT NULL;

            CREATE INDEX IF NOT EXISTS idx_sessions_agent_role
                ON sessions(agent_id, role);

            -- Entity types table (seed data for 11 SAG entity types)
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
                (11, 'tags',          '标签',     '兜底标签，当无特定类型匹配时使用', 0.5, 0.70, 1, 1);

            -- Entities table (per-agent entity storage)
            CREATE TABLE IF NOT EXISTS entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                normalized_name TEXT NOT NULL,
                description TEXT,
                UNIQUE(agent_id, type, normalized_name)
            );

            CREATE INDEX IF NOT EXISTS idx_entities_agent_normalized
                ON entities(agent_id, normalized_name);

            -- Event-entity association table
            CREATE TABLE IF NOT EXISTS event_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id INTEGER NOT NULL,
                entity_id INTEGER NOT NULL,
                FOREIGN KEY (entity_id) REFERENCES entities(id)
            );

            CREATE INDEX IF NOT EXISTS idx_event_entities_entity_id
                ON event_entities(entity_id);

            CREATE INDEX IF NOT EXISTS idx_event_entities_event_id
                ON event_entities(event_id);
            "#,
        )
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

        for (col, col_type) in [
            ("thread_id", "TEXT"),
            ("sender_id", "TEXT"),
            ("platform", "TEXT"),
            ("peer_id", "TEXT"),
            ("account_id", "TEXT"),
            ("parent_session_id", "TEXT"),
            ("depth", "TEXT"),
            ("mined", "TEXT"),
            ("dreaming_status", "TEXT"),
            ("plan_state", "TEXT"),
            ("mined_at", "INTEGER"),
        ] {
            Self::add_column_if_not_exists(conn, col, col_type)?;
        }

        Ok(())
    }
    /// Returns the data directory path
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// List IDs of active sessions idle for at least `idle_minutes`.
    pub async fn list_idle_sessions(
        &self,
        idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        spawn_blocking(move || archive_support::list_idle_sessions_inner(&data_dir, idle_minutes))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List IDs of archived sessions past their purge window.
    pub async fn list_expired_archived_sessions(
        &self,
        purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        spawn_blocking(move || {
            archive_support::list_expired_archived_sessions_inner(&data_dir, purge_after_minutes)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List IDs of active sessions for a specific agent/role idle for at least
    /// `idle_minutes`.  `role` is a string such as "main_agent" or "sub_agent".
    pub async fn list_idle_sessions_for_agent(
        &self,
        agent_id: &str,
        role: &str,
        idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let agent_id = agent_id.to_string();
        let role = role.to_string();
        spawn_blocking(move || {
            archive_support::list_idle_sessions_for_agent_inner(
                &data_dir,
                &agent_id,
                &role,
                idle_minutes,
            )
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List IDs of archived sessions for a specific agent/role past their purge
    /// window.  `role` is a string such as "main_agent" or "sub_agent".
    pub async fn list_expired_archived_sessions_for_agent(
        &self,
        agent_id: &str,
        role: &str,
        purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let agent_id = agent_id.to_string();
        let role = role.to_string();
        spawn_blocking(move || {
            archive_support::list_expired_archived_sessions_for_agent_inner(
                &data_dir,
                &agent_id,
                &role,
                purge_after_minutes,
            )
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }
}

impl std::fmt::Display for SqliteStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SqliteStorage({})", self.data_dir.display())
    }
}

impl Clone for SqliteStorage {
    fn clone(&self) -> Self {
        Self {
            data_dir: self.data_dir.clone(),
        }
    }
}

/// Convert SessionStatus to/from database string representation
fn status_to_db(s: &crate::persistence::SessionStatus) -> &'static str {
    match s {
        crate::persistence::SessionStatus::Active => "active",
        crate::persistence::SessionStatus::Archived => "archived",
    }
}

/// Convert ReasonMode to/from database string representation
fn mode_to_db(m: &crate::persistence::ReasoningMode) -> &'static str {
    match m {
        crate::persistence::ReasoningMode::Direct => "direct",
        crate::persistence::ReasoningMode::Plan => "plan",
        crate::persistence::ReasoningMode::Stream => "stream",
        crate::persistence::ReasoningMode::Hidden => "hidden",
    }
}

impl SqliteStorage {
    /// Load a SessionCheckpoint from an open DB connection.
    /// Used by `load_checkpoint`.
    fn load_checkpoint_inner(
        conn: &Connection,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        archive_support::load_checkpoint_inner(conn, data_dir, session_id)
    }

    /// Find an active session by routing fields on an open connection.
    fn find_active_session_inner(
        conn: &Connection,
        channel: &str,
        sender_id: &str,
        peer_id: &str,
        account_id: Option<&str>,
    ) -> Result<Option<String>, PersistenceError> {
        let result = if let Some(acc) = account_id {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM sessions
                     WHERE status = 'active'
                       AND platform = ?1
                       AND sender_id = ?2
                       AND peer_id = ?3
                       AND account_id = ?4",
                )
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            stmt.query_row(params![channel, sender_id, peer_id, acc], |row| {
                row.get::<_, String>(0)
            })
            .ok()
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM sessions
                     WHERE status = 'active'
                       AND platform = ?1
                       AND sender_id = ?2
                       AND peer_id = ?3
                       AND account_id IS NULL",
                )
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            stmt.query_row(params![channel, sender_id, peer_id], |row| {
                row.get::<_, String>(0)
            })
            .ok()
        };
        Ok(result)
    }
}

#[async_trait]
impl PersistenceService for SqliteStorage {
    /// Save a session checkpoint to the database and write its transcript
    /// to `sessions/<id>.jsonl`.
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let checkpoint = checkpoint.clone();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let status = status_to_db(&checkpoint.status);
            let mode_state_json = serde_json::to_string(&checkpoint.mode_state)
                .map_err(PersistenceError::Serialization)?;
            let pending_json = serde_json::to_string(&checkpoint.outbound_pending)
                .map_err(PersistenceError::Serialization)?;
            let system_appends_json = serde_json::to_string(&checkpoint.system_appends)
                .map_err(PersistenceError::Serialization)?;
            let transcript_json = serde_json::to_string(&checkpoint.pending_messages)
                .map_err(PersistenceError::Serialization)?;
            let metadata_json = json!({
                "mode": mode_to_db(&checkpoint.mode),
                "mode_state": mode_state_json,
                "outbound_pending": pending_json,
                "system_appends": system_appends_json,
                "session_mode": checkpoint.session_mode.to_string(),
                "transcript": transcript_json,
            })
            .to_string();

            let last_msg_ts = checkpoint
                .last_message_at
                .map(|dt| dt.timestamp())
                .unwrap_or(0);

            let dreaming_status_str =
                crate::persistence::dreaming_status_to_db(&checkpoint.dreaming_status);
            let mined_str = if checkpoint.mined { "1" } else { "0" };
            let mined_at_val = checkpoint.mined_at;
            let plan_state_json = checkpoint
                .plan_state
                .as_ref()
                .map(|ps| serde_json::to_string(ps).map_err(PersistenceError::Serialization))
                .transpose()?;

            conn.execute(
                "INSERT OR REPLACE INTO sessions
                 (id, agent_id, role, channel, chat_id, status, title,
                  last_message_at, created_at, archived_at, message_count, metadata, thread_id,
                  sender_id, platform, peer_id, account_id, parent_session_id, depth,
                  mined, dreaming_status, plan_state, mined_at)
                 VALUES (
                     ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                     ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19,
                     ?20, ?21, ?22, ?23
                 )",
                params![
                    checkpoint.session_id,
                    checkpoint.agent_id.as_deref().unwrap_or("unknown"),
                    checkpoint
                        .role
                        .map(|r| match r {
                            crate::persistence::AgentRole::MainAgent => "main_agent",
                            crate::persistence::AgentRole::SubAgent => "sub_agent",
                        })
                        .unwrap_or("main_agent"),
                    // Backward-compat: write platform value to old channel column
                    checkpoint.platform.as_deref().unwrap_or(""),
                    // Backward-compat: write peer_id value to old chat_id column
                    checkpoint.peer_id.as_deref().unwrap_or(""),
                    status,
                    Option::<&str>::None, // title
                    last_msg_ts,
                    checkpoint.created_at.timestamp(),
                    Option::<i64>::None, // archived_at
                    checkpoint.message_count as i64,
                    metadata_json,
                    checkpoint.thread_id.as_deref(),
                    checkpoint.sender_id.as_deref(),
                    // New columns
                    checkpoint.platform.as_deref(),
                    checkpoint.peer_id.as_deref(),
                    checkpoint.account_id.as_deref(),
                    checkpoint.parent_session_id.as_deref(),
                    checkpoint.depth,
                    mined_str,
                    dreaming_status_str,
                    plan_state_json.as_deref(),
                    mined_at_val,
                ],
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            // Write transcript to sessions/<id>.jsonl
            let transcript_path = data_dir
                .join("sessions")
                .join(format!("{}.jsonl", checkpoint.session_id));
            archive_support::write_transcript(&transcript_path, &checkpoint)?;

            Ok(())
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Load a session checkpoint from the database and reconstruct
    /// outbound_pending from the transcript file.
    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            Self::load_checkpoint_inner(&conn, &data_dir, &session_id)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    async fn load_archived_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            Self::load_checkpoint_inner(&conn, &data_dir, &session_id)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Delete a session checkpoint from the database and remove its
    /// transcript file (active or archived).
    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            conn.execute(
                "DELETE FROM sessions WHERE id = ?1",
                rusqlite::params![session_id],
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            // Remove transcript from sessions/ then archived_sessions/
            let active_path = data_dir
                .join("sessions")
                .join(format!("{session_id}.jsonl"));
            archive_support::delete_transcript(&active_path);

            let archived_path = data_dir
                .join("archived_sessions")
                .join(format!("{session_id}.jsonl"));
            archive_support::delete_transcript(&archived_path);

            Ok(())
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List all active session IDs.
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let mut stmt = conn
                .prepare("SELECT id FROM sessions WHERE status = 'active'")
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let ids: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();

            Ok(ids)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Find an active session matching the given routing fields.
    ///
    /// When `account_id` is `None`, matches rows where `account_id IS NULL`.
    async fn find_active_session_by_routing(
        &self,
        account_id: Option<&str>,
        channel: &str,
        sender_id: &str,
        peer_id: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let channel = channel.to_string();
        let sender_id = sender_id.to_string();
        let peer_id = peer_id.to_string();
        let account_id = account_id.map(String::from);

        spawn_blocking(move || -> Result<Option<String>, PersistenceError> {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            Self::find_active_session_inner(
                &conn,
                &channel,
                &sender_id,
                &peer_id,
                account_id.as_deref(),
            )
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Archive a session: move its transcript to archived_sessions/ and mark
    /// the DB record as archived. Idempotent if the session is not active.
    async fn archive_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let checkpoint = checkpoint.clone();

        spawn_blocking(move || archive_support::do_archive(&data_dir, &checkpoint))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Restore an archived session: move transcript back to sessions/ and mark
    /// the DB record as active.
    async fn restore_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || archive_support::do_restore(&data_dir, &session_id))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Permanently delete an archived checkpoint and its transcript.
    async fn purge_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || archive_support::do_purge(&data_dir, &session_id))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List all archived session IDs.
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();

        spawn_blocking(move || archive_support::do_list_archived(&data_dir))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Invalidate a session (no-op for SQLite backend).
    async fn invalidate_session(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn sync(&self) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Close the storage backend (no-op for SQLite).
    ///
    /// SqliteStorage opens temporary connections per operation and closes
    /// them immediately — no persistent connection or file handle to release.
    /// This method provides the explicit close interface for shutdown phase 6
    /// while remaining a no-op for correctness.
    async fn close(&self) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_idle_sessions_for_agent(
        &self,
        agent_id: &str,
        role: crate::persistence::AgentRole,
        idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let role_str = match role {
            crate::persistence::AgentRole::MainAgent => "main_agent",
            crate::persistence::AgentRole::SubAgent => "sub_agent",
        };
        self.list_idle_sessions_for_agent(agent_id, role_str, idle_minutes)
            .await
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        agent_id: &str,
        role: crate::persistence::AgentRole,
        purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let role_str = match role {
            crate::persistence::AgentRole::MainAgent => "main_agent",
            crate::persistence::AgentRole::SubAgent => "sub_agent",
        };
        self.list_expired_archived_sessions_for_agent(agent_id, role_str, purge_after_minutes)
            .await
    }

    async fn list_children_sessions(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let parent_id = parent_session_id.to_string();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let mut stmt = conn
                .prepare("SELECT id FROM sessions WHERE parent_session_id = ?1")
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let ids: Vec<String> = stmt
                .query_map(rusqlite::params![parent_id], |row| row.get(0))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();

            Ok(ids)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let mut stmt = conn
                .prepare("SELECT id FROM sessions WHERE status = 'archived' AND mined = 0")
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let ids: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();

            Ok(ids)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let mut stmt = conn
                .prepare(
                    "SELECT id FROM sessions \
                     WHERE mined = 1 AND dreaming_status != 'completed'",
                )
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let ids: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();

            Ok(ids)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    async fn mark_mined(&self, session_id: &str) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();
        let now = chrono::Utc::now().timestamp();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            conn.execute(
                "UPDATE sessions SET mined = 1, mined_at = ?1 WHERE id = ?2",
                rusqlite::params![now, session_id],
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            Ok(())
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    async fn update_dreaming_status(
        &self,
        session_id: &str,
        status: crate::persistence::DreamingStatus,
    ) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();
        let status_str = crate::persistence::dreaming_status_to_db(&status).to_string();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            conn.execute(
                "UPDATE sessions SET dreaming_status = ?1 WHERE id = ?2",
                rusqlite::params![status_str, session_id],
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            Ok(())
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    async fn run_consistency_check(&self) -> Result<ConsistencyCheckResult, PersistenceError> {
        let data_dir = self.data_dir.clone();

        spawn_blocking(move || {
            let mut result = ConsistencyCheckResult::default();
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            check_sqlite_to_filesystem(&conn, &data_dir, &mut result)?;
            check_filesystem_to_sqlite(&conn, &data_dir, &mut result)?;

            Ok(result)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }
}

/// SQLite → File system: remove active records whose transcript is missing.
fn check_sqlite_to_filesystem(
    conn: &Connection,
    data_dir: &std::path::Path,
    result: &mut ConsistencyCheckResult,
) -> Result<(), PersistenceError> {
    let active_ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT id FROM sessions WHERE status = 'active'")
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        ids
    };

    for id in &active_ids {
        let transcript_path = data_dir.join("sessions").join(format!("{id}.jsonl"));
        if !transcript_path.exists() {
            conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            result.deleted_orphaned_records += 1;
        }
    }
    Ok(())
}

/// File system → SQLite: remove orphan transcript files not in SQLite.
fn check_filesystem_to_sqlite(
    conn: &Connection,
    data_dir: &std::path::Path,
    result: &mut ConsistencyCheckResult,
) -> Result<(), PersistenceError> {
    let sessions_dir = data_dir.join("sessions");
    if !sessions_dir.exists() {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(&sessions_dir).map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    {
        let entry = entry.map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let file_stem = path.file_stem().and_then(|e| e.to_str()).unwrap_or("");
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sessions WHERE id = ?1",
                rusqlite::params![file_stem],
                |row| row.get(0),
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        if !exists {
            std::fs::remove_file(&path).map_err(PersistenceError::Io)?;
            result.deleted_orphaned_files += 1;
        }
    }
    Ok(())
}
