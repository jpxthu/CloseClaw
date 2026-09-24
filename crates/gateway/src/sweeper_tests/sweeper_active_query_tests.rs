//! ActiveSessionQuery integration tests for ArchiveSweeper.
//!
//! Verifies that the sweeper correctly skips archiving sessions that are
//! actively executing work (LLM call, tool execution, etc.).

#[cfg(test)]
mod tests {
    use crate::sweeper::*;
    use async_trait::async_trait;
    use closeclaw_common::SessionActivityDimensions;
    use closeclaw_config::SessionConfigProvider;
    use closeclaw_session::persistence::{PersistenceService, SessionCheckpoint};
    use std::sync::{Arc, Mutex};

    use super::super::sweeper_test_utils::{MemStorage, MockConfig};

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
}
