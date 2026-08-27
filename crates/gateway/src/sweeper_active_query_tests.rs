//! Step 1.6: ActiveSessionQuery integration tests for ArchiveSweeper.
//!
//! Verifies that the sweeper correctly skips archiving sessions that are
//! actively executing work (LLM call, tool execution, etc.).

#[cfg(test)]
mod tests {
    use crate::sweeper::*;
    use async_trait::async_trait;
    use closeclaw_common::SessionActivityDimensions;
    use closeclaw_config::session::PerAgentSessionConfig;
    use closeclaw_config::SessionConfigProvider;
    use closeclaw_session::persistence::{
        AgentRole, PersistenceError, PersistenceService, SessionCheckpoint,
    };
    use std::sync::{Arc, Mutex};

    /// In-memory storage suitable for tests.
    #[derive(Debug, Default)]
    struct MemStorage {
        checkpoints: Mutex<Vec<SessionCheckpoint>>,
        invalidated: Mutex<Vec<String>>,
        archive_called: Mutex<Vec<String>>,
        purge_called: Mutex<Vec<String>>,
        idle_sessions: Mutex<Vec<String>>,
        expired_sessions: Mutex<Vec<String>>,
        deleted: Mutex<Vec<String>>,
    }

    impl MemStorage {
        fn add_idle_session(&self, session_id: String) {
            self.idle_sessions.lock().unwrap().push(session_id);
        }

        fn add_checkpoint(&self, checkpoint: SessionCheckpoint) {
            self.checkpoints.lock().unwrap().push(checkpoint);
        }
    }

    #[async_trait]
    impl PersistenceService for MemStorage {
        async fn save_checkpoint(
            &self,
            checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            self.checkpoints.lock().unwrap().push(checkpoint.clone());
            Ok(())
        }

        async fn load_checkpoint(
            &self,
            session_id: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            let checkpoints = self.checkpoints.lock().unwrap();
            Ok(checkpoints
                .iter()
                .find(|cp| cp.session_id == session_id)
                .cloned())
        }

        async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.deleted.lock().unwrap().push(session_id.into());
            self.checkpoints
                .lock()
                .unwrap()
                .retain(|cp| cp.session_id != session_id);
            Ok(())
        }

        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }

        async fn archive_checkpoint(
            &self,
            checkpoint: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            self.archive_called
                .lock()
                .unwrap()
                .push(checkpoint.session_id.clone());
            Ok(())
        }

        async fn purge_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.purge_called.lock().unwrap().push(session_id.into());
            Ok(())
        }

        async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }

        async fn invalidate_session(&self, session_id: &str) -> Result<(), PersistenceError> {
            self.invalidated.lock().unwrap().push(session_id.into());
            Ok(())
        }

        async fn list_idle_sessions_for_agent(
            &self,
            _agent_id: &str,
            _role: AgentRole,
            _idle_minutes: i64,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(self.idle_sessions.lock().unwrap().clone())
        }

        async fn list_expired_archived_sessions_for_agent(
            &self,
            _agent_id: &str,
            _role: AgentRole,
            _purge_after_minutes: i64,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(self.expired_sessions.lock().unwrap().clone())
        }

        async fn list_children_sessions(
            &self,
            _parent_session_id: &str,
        ) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
    }

    /// Mock config provider for tests.
    #[derive(Debug, Default)]
    struct MockConfig {
        agents: Mutex<Vec<String>>,
        session_config: Mutex<PerAgentSessionConfig>,
        interval_secs: Mutex<u64>,
    }

    impl MockConfig {
        fn with_agents(agents: Vec<String>) -> Self {
            Self {
                agents: Mutex::new(agents),
                ..Default::default()
            }
        }
    }

    impl SessionConfigProvider for MockConfig {
        fn session_config_for(&self, _agent_id: &str, _role: AgentRole) -> PerAgentSessionConfig {
            self.session_config.lock().unwrap().clone()
        }

        fn sweeper_interval_secs(&self) -> u64 {
            *self.interval_secs.lock().unwrap()
        }

        fn dreaming_interval_secs(&self) -> u64 {
            600
        }

        fn consistency_check_interval_secs(&self) -> u64 {
            3600
        }

        fn list_agents(&self) -> Vec<String> {
            self.agents.lock().unwrap().clone()
        }

        fn compact_config(&self) -> closeclaw_common::CompactConfig {
            closeclaw_common::CompactConfig::default()
        }
    }

    /// Mock ActiveSessionQuery that returns active dimensions for specified session IDs.
    struct MockActiveQuery {
        active_ids: Mutex<Vec<String>>,
    }

    impl MockActiveQuery {
        fn new(active_ids: Vec<String>) -> Self {
            Self {
                active_ids: Mutex::new(active_ids),
            }
        }

        fn none() -> Self {
            Self::new(vec![])
        }
    }

    #[async_trait]
    impl ActiveSessionQuery for MockActiveQuery {
        async fn activity_dimensions(&self, session_id: &str) -> SessionActivityDimensions {
            if self
                .active_ids
                .lock()
                .unwrap()
                .contains(&session_id.to_string())
            {
                // Return all-active dimensions — the consumer checks any_active()
                SessionActivityDimensions {
                    llm_active: true,
                    foreground_tool_active: true,
                    background_tool_active: true,
                    child_active: true,
                }
            } else {
                SessionActivityDimensions::default()
            }
        }
    }

    // ── Tests ────────────────────────────────────────────────────────

    /// With active_query injected, session marked active is NOT archived.
    #[tokio::test]
    async fn test_sweeper_skips_active_session() {
        let mem = Arc::new(MemStorage::default());
        mem.add_idle_session("active-sess".into());
        mem.add_checkpoint(SessionCheckpoint::new("active-sess".into()));

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> =
            Arc::new(MockConfig::with_agents(vec!["agent-x".into()]));

        let active_query: Arc<dyn ActiveSessionQuery> =
            Arc::new(MockActiveQuery::new(vec!["active-sess".into()]));

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config))
            .with_active_query(active_query);
        sweeper.run_once().await.unwrap();

        let archive_called = mem.archive_called.lock().unwrap();
        assert!(
            !archive_called.contains(&"active-sess".into()),
            "session reported as active should NOT be archived"
        );
    }

    /// Without active_query (None), inactive session IS archived (baseline).
    #[tokio::test]
    async fn test_sweeper_archives_inactive_session_no_active_query() {
        let mem = Arc::new(MemStorage::default());
        mem.add_idle_session("inactive-sess".into());
        mem.add_checkpoint(SessionCheckpoint::new("inactive-sess".into()));

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> =
            Arc::new(MockConfig::with_agents(vec!["agent-x".into()]));

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config));
        sweeper.run_once().await.unwrap();

        let archive_called = mem.archive_called.lock().unwrap();
        assert!(
            archive_called.contains(&"inactive-sess".into()),
            "session with no active_query should be archived"
        );
    }

    /// With active_query injected, session NOT in active list IS archived.
    #[tokio::test]
    async fn test_sweeper_archives_inactive_session_with_active_query() {
        let mem = Arc::new(MemStorage::default());
        mem.add_idle_session("not-active-sess".into());
        mem.add_checkpoint(SessionCheckpoint::new("not-active-sess".into()));

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> =
            Arc::new(MockConfig::with_agents(vec!["agent-x".into()]));

        let active_query: Arc<dyn ActiveSessionQuery> = Arc::new(MockActiveQuery::none());

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config))
            .with_active_query(active_query);
        sweeper.run_once().await.unwrap();

        let archive_called = mem.archive_called.lock().unwrap();
        assert!(
            archive_called.contains(&"not-active-sess".into()),
            "session not in active list should be archived"
        );
    }

    /// Multiple idle sessions: active one skipped, inactive one archived.
    #[tokio::test]
    async fn test_sweeper_mixed_active_and_inactive_sessions() {
        let mem = Arc::new(MemStorage::default());
        mem.add_idle_session("mixed-active".into());
        mem.add_idle_session("mixed-inactive".into());
        mem.add_checkpoint(SessionCheckpoint::new("mixed-active".into()));
        mem.add_checkpoint(SessionCheckpoint::new("mixed-inactive".into()));

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> =
            Arc::new(MockConfig::with_agents(vec!["agent-x".into()]));

        let active_query: Arc<dyn ActiveSessionQuery> =
            Arc::new(MockActiveQuery::new(vec!["mixed-active".into()]));

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config))
            .with_active_query(active_query);
        sweeper.run_once().await.unwrap();

        let archive_called = mem.archive_called.lock().unwrap();
        assert!(
            !archive_called.contains(&"mixed-active".into()),
            "active session should be skipped"
        );
        assert!(
            archive_called.contains(&"mixed-inactive".into()),
            "inactive session should be archived"
        );
    }

    /// Pending operations + active query: pending_operations check happens
    /// first, active query check happens after. Session with pending ops
    /// is skipped regardless of active query.
    #[tokio::test]
    async fn test_sweeper_pending_operations_checked_before_active_query() {
        use chrono::Utc;
        use closeclaw_session::persistence::{
            PendingOperation, PendingOperationStatus, PendingOperationType,
        };

        let mem = Arc::new(MemStorage::default());
        mem.add_idle_session("pend-and-active".into());

        let mut cp = SessionCheckpoint::new("pend-and-active".into());
        cp = cp.with_pending_operations(vec![PendingOperation {
            op_id: "op-1".into(),
            op_type: PendingOperationType::ToolCall,
            status: PendingOperationStatus::Running,
            detail: closeclaw_session::persistence::PendingOperationDetail::ToolCall {
                tool_name: "bash".into(),
                args_summary: "{}".into(),
            },
            created_at: Utc::now(),
        }]);
        mem.add_checkpoint(cp);

        let storage: Arc<dyn PersistenceService> = mem.clone() as _;
        let config: Arc<dyn SessionConfigProvider> =
            Arc::new(MockConfig::with_agents(vec!["agent-x".into()]));

        let active_query: Arc<dyn ActiveSessionQuery> = Arc::new(MockActiveQuery::none());

        let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&config))
            .with_active_query(active_query);
        sweeper.run_once().await.unwrap();

        let archive_called = mem.archive_called.lock().unwrap();
        assert!(
            !archive_called.contains(&"pend-and-active".into()),
            "session with pending_operations should be skipped regardless of active_query"
        );
    }
}
