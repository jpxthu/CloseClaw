//! Shared fixtures for sweeper tests: in-memory persistence storage plus
//! config, active-query and task-manager mocks used across the sweeper
//! test modules.

use async_trait::async_trait;
use closeclaw_common::SessionActivityDimensions;
use closeclaw_config::session::PerAgentSessionConfig;
use closeclaw_config::SessionConfigProvider;
use closeclaw_session::persistence::{
    AgentRole, PersistenceError, PersistenceService, SessionCheckpoint,
};
use closeclaw_tasks::{BackgroundTask, BackgroundTaskError, CompletionNotification, TaskManager};
use std::sync::{Arc, Mutex};

use crate::sweeper::ActiveSessionQuery;

/// In-memory storage suitable for tests.
#[derive(Debug, Default)]
pub struct MemStorage {
    checkpoints: Mutex<Vec<SessionCheckpoint>>,
    _archived: Mutex<Vec<String>>,
    invalidated: Mutex<Vec<String>>,
    pub archive_called: Mutex<Vec<String>>,
    pub purge_called: Mutex<Vec<String>>,
    /// Session IDs returned from `list_idle_sessions_for_agent`.
    idle_sessions: Mutex<Vec<String>>,
    /// Session IDs returned from `list_expired_archived_sessions_for_agent`.
    expired_sessions: Mutex<Vec<String>>,
    /// Session IDs deleted via `delete_checkpoint`.
    deleted: Mutex<Vec<String>>,
}

impl MemStorage {
    /// Add a session ID to be returned as idle by `list_idle_sessions_for_agent`.
    pub fn add_idle_session(&self, session_id: String) {
        self.idle_sessions.lock().unwrap().push(session_id);
    }

    /// Add a session ID to be returned as expired by
    /// `list_expired_archived_sessions_for_agent`.
    pub fn add_expired_session(&self, session_id: String) {
        self.expired_sessions.lock().unwrap().push(session_id);
    }

    /// Add a checkpoint so it can be loaded by `load_checkpoint`.
    pub fn add_checkpoint(&self, checkpoint: SessionCheckpoint) {
        self.checkpoints.lock().unwrap().push(checkpoint);
    }

    /// Get IDs of sessions deleted via `delete_checkpoint`.
    pub fn deleted_ids(&self) -> Vec<String> {
        self.deleted.lock().unwrap().clone()
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
        parent_session_id: &str,
    ) -> Result<Vec<String>, PersistenceError> {
        let checkpoints = self.checkpoints.lock().unwrap();
        let children: Vec<String> = checkpoints
            .iter()
            .filter(|cp| cp.parent_session_id.as_deref() == Some(parent_session_id))
            .map(|cp| cp.session_id.clone())
            .collect();
        Ok(children)
    }
}

/// Mock config provider for tests.
#[derive(Debug, Default)]
pub struct MockConfig {
    agents: Mutex<Vec<String>>,
    pub session_config: Mutex<PerAgentSessionConfig>,
    interval_secs: Mutex<u64>,
}

impl MockConfig {
    pub fn with_agents(agents: Vec<String>) -> Self {
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

    fn plan_archive_days(&self) -> u64 {
        7
    }

    fn audit_log_limit(&self) -> usize {
        1000
    }
}

/// Mock ActiveSessionQuery that returns all-false (no active dimensions).
pub struct MockActiveQuery;

impl MockActiveQuery {
    pub fn none() -> Self {
        Self
    }
}

#[async_trait]
impl ActiveSessionQuery for MockActiveQuery {
    async fn activity_dimensions(&self, _session_id: &str) -> SessionActivityDimensions {
        SessionActivityDimensions::default()
    }
}

/// Mock ActiveSessionQuery that returns custom dimensions per session ID.
pub struct MockActiveQueryWithDimensions {
    dimensions: Mutex<Vec<(String, SessionActivityDimensions)>>,
}

impl MockActiveQueryWithDimensions {
    pub fn new(dimensions: Vec<(String, SessionActivityDimensions)>) -> Self {
        Self {
            dimensions: Mutex::new(dimensions),
        }
    }
}

#[async_trait]
impl ActiveSessionQuery for MockActiveQueryWithDimensions {
    async fn activity_dimensions(&self, session_id: &str) -> SessionActivityDimensions {
        self.dimensions
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| id == session_id)
            .map(|(_, d)| *d)
            .unwrap_or_default()
    }
}

/// Mock TaskManager that tracks `cleanup_all_finished` calls.
pub struct MockTaskManager {
    cleanup_all_finished_called: Arc<Mutex<bool>>,
    session_id_arg: Arc<Mutex<Option<String>>>,
}

impl MockTaskManager {
    pub fn new() -> (Self, Arc<Mutex<bool>>, Arc<Mutex<Option<String>>>) {
        let called = Arc::new(Mutex::new(false));
        let sid = Arc::new(Mutex::new(None));
        (
            Self {
                cleanup_all_finished_called: Arc::clone(&called),
                session_id_arg: Arc::clone(&sid),
            },
            called,
            sid,
        )
    }
}

#[async_trait]
impl TaskManager for MockTaskManager {
    async fn spawn_task(
        &self,
        _command: &str,
        _cwd: &std::path::Path,
        _is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        unimplemented!()
    }
    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        _command: &str,
        _is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        unimplemented!()
    }
    async fn kill_task(&self, _: &str) -> Result<(), BackgroundTaskError> {
        unimplemented!()
    }
    async fn get_task(&self, _: &str) -> Option<BackgroundTask> {
        unimplemented!()
    }
    async fn list_running_tasks(&self) -> Vec<closeclaw_tasks::RunningTaskInfo> {
        unimplemented!()
    }
    async fn drain_notifications(&self) -> Vec<CompletionNotification> {
        unimplemented!()
    }
    async fn cleanup_all_finished(&self, session_id: &str) {
        *self.cleanup_all_finished_called.lock().unwrap() = true;
        *self.session_id_arg.lock().unwrap() = Some(session_id.to_owned());
    }
    fn max_execution_secs(&self) -> u64 {
        3600
    }
}
