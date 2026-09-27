//! SQLite storage backend for session persistence
//!
//! This backend stores session checkpoints in a local SQLite database,
//! suitable for single-node deployments without Redis.

mod archive_support;
mod consistency_check;
mod schema;
mod snapshot_meta_support;

#[cfg(test)]
mod bug904_tests;
#[cfg(test)]
mod consistency_check_tests;
#[cfg(test)]
mod migrating_archive_tests;
#[cfg(test)]
mod snapshot_meta_tests;
#[cfg(test)]
mod tests;

use crate::persistence::{
    ConsistencyCheckResult, PersistenceError, PersistenceService, SessionCheckpoint,
};
use crate::run_health::SnapshotMeta;
use async_trait::async_trait;
use rusqlite::{params, Connection};
#[cfg(test)]
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
        schema::init_schema(&conn)?;

        Ok(Self { data_dir })
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

    /// List IDs of sessions in `migrating` status.
    pub async fn list_migrating_sessions_inner(&self) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            let mut stmt = conn
                .prepare("SELECT id FROM sessions WHERE status = 'migrating'")
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

/// Convert ReasonMode to/from database string representation
#[cfg(test)]
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

    /// Find a migrating session by routing fields on an open connection.
    fn find_migrating_session_inner(
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
                     WHERE status = 'migrating'
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
                     WHERE status = 'migrating'
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

    /// Find an archived session by routing fields on an open connection.
    ///
    /// Returns the session with the most recent `last_message_at`.
    fn find_archived_session_inner(
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
                     WHERE status = 'archived'
                       AND platform = ?1
                       AND sender_id = ?2
                       AND peer_id = ?3
                       AND account_id = ?4
                     ORDER BY last_message_at DESC
                     LIMIT 1",
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
                     WHERE status = 'archived'
                       AND platform = ?1
                       AND sender_id = ?2
                       AND peer_id = ?3
                       AND account_id IS NULL
                     ORDER BY last_message_at DESC
                     LIMIT 1",
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

        spawn_blocking(move || archive_support::save_checkpoint_inner(&data_dir, &checkpoint))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Load a session checkpoint from the database. Transcript is read from
    /// the JSONL file; outbound_pending from metadata JSON.
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

    /// Find a migrating session matching the given routing fields.
    ///
    /// When `account_id` is `None`, matches rows where `account_id IS NULL`.
    async fn find_migrating_session_by_routing(
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
            Self::find_migrating_session_inner(
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

    /// Find an archived session matching the given routing fields.
    ///
    /// Returns the archived session with the most recent `last_message_at`.
    /// When `account_id` is `None`, matches rows where `account_id IS NULL`.
    async fn find_archived_session_by_routing(
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
            Self::find_archived_session_inner(
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

    async fn list_migrating_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        self.list_migrating_sessions_inner().await
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

    async fn save_snapshot_metas(
        &self,
        session_id: &str,
        metas: &[SnapshotMeta],
    ) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();
        let metas = metas.to_vec();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            snapshot_meta_support::save_snapshot_metas_inner(&conn, &session_id, &metas)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    async fn load_snapshot_metas(
        &self,
        session_id: &str,
    ) -> Result<Vec<SnapshotMeta>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            snapshot_meta_support::load_snapshot_metas_inner(&conn, &session_id)
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
        // Full scan == incremental scan since epoch start (since = 0)
        self.run_incremental_consistency_check(0).await
    }

    async fn run_incremental_consistency_check(
        &self,
        since: i64,
    ) -> Result<ConsistencyCheckResult, PersistenceError> {
        let data_dir = self.data_dir.clone();
        spawn_blocking(move || {
            let mut result = ConsistencyCheckResult::default();
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            consistency_check::check_sqlite_to_filesystem_filtered(
                &conn,
                &data_dir,
                &mut result,
                since,
            )?;
            consistency_check::check_filesystem_to_sqlite_filtered(
                &conn,
                &data_dir,
                &mut result,
                since,
            )?;
            Ok(result)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }
}
