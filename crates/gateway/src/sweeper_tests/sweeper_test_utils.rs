//! Shared fixtures for sweeper tests: in-memory persistence storage plus
//! config, active-query and task-manager mocks used across the sweeper
//! test modules.

use async_trait::async_trait;
use closeclaw_common::SessionActivityDimensions;
use closeclaw_config::session::{PerAgentSessionConfig, DEFAULT_SWEEPER_INTERVAL_SECS};
use closeclaw_config::SessionConfigProvider;
use closeclaw_session::persistence::{
    AgentRole, PersistenceError, PersistenceService, SessionCheckpoint,
};
use closeclaw_tasks::{BackgroundTask, BackgroundTaskError, CompletionNotification, TaskManager};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::sweeper::{ActiveSessionQuery, ArchiveSweeper};

/// Fault to inject into [`MemStorage`]'s `list_idle_sessions_for_agent`
/// calls so tests can drive the sweeper's real error/panic handling
/// branches without touching production code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFault {
    /// Return `PersistenceError::Io` — swallowed by `sweep_agent_role`'s
    /// `if let Ok(...)`, the sweep loop must continue.
    Err,
    /// Panic inside the storage call — caught by `run_once`'s
    /// `catch_unwind`, which turns it into `Ok(())`.
    Panic,
}

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
    /// Total `list_idle_sessions_for_agent` calls observed (fault probe).
    list_idle_calls: AtomicUsize,
    /// Pending injected faults for `list_idle_sessions_for_agent`.
    list_idle_fault: Mutex<Option<(StorageFault, usize)>>,
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

    /// Arm `count` faults on the next `list_idle_sessions_for_agent`
    /// calls: each armed call either returns `Err` or panics (see
    /// [`StorageFault`]), letting tests execute the production error
    /// branches for real.
    ///
    /// `count` is exact: the fault fires on at most the next `count`
    /// calls and then stops. `count == 0` arms nothing and disarms any
    /// previously armed fault — it never triggers.
    pub fn inject_list_idle_fault(&self, fault: StorageFault, count: usize) {
        let mut guard = self.list_idle_fault.lock().unwrap();
        *guard = if count == 0 {
            None
        } else {
            Some((fault, count))
        };
    }

    /// Number of `list_idle_sessions_for_agent` calls so far — proves the
    /// injected faults actually executed (and how far the sweep got).
    pub fn list_idle_calls(&self) -> usize {
        self.list_idle_calls.load(Ordering::SeqCst)
    }

    /// Pop the next pending fault, if any. The fault mutex guard is
    /// released before the caller panics, so it is never poisoned.
    fn take_list_idle_fault(&self) -> Option<StorageFault> {
        let mut guard = self.list_idle_fault.lock().unwrap();
        let (fault, remaining) = (*guard)?;
        if remaining <= 1 {
            *guard = None;
        } else {
            *guard = Some((fault, remaining - 1));
        }
        Some(fault)
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
        self.list_idle_calls.fetch_add(1, Ordering::SeqCst);
        match self.take_list_idle_fault() {
            Some(StorageFault::Err) => Err(PersistenceError::Io(std::io::Error::other(
                "injected list_idle failure (sweeper error-path test)",
            ))),
            Some(StorageFault::Panic) => {
                panic!("injected list_idle panic (sweeper catch_unwind test)")
            }
            None => Ok(self.idle_sessions.lock().unwrap().clone()),
        }
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
        // Production default, never 0: `ArchiveSweeper::run` derives
        // `next_fire` from this, and a zero would degenerate into a
        // busy-fire loop instead of a periodic tick.
        DEFAULT_SWEEPER_INTERVAL_SECS
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

// ── sweeper construction helpers ──────────────────────────────────

/// Build an [`ArchiveSweeper`] over shared fixtures: fresh in-memory
/// storage plus a [`MockConfig`] seeded with `agents` and the default
/// session config. Returns the storage so cases can assert on the
/// recorded `archive_called` / `purge_called` calls.
pub fn sweeper_with_agents(agents: Vec<String>) -> (Arc<MemStorage>, ArchiveSweeper) {
    sweeper_with_session_config(agents, PerAgentSessionConfig::default())
}

/// Like [`sweeper_with_agents`], but with an explicit session config —
/// e.g. a non-zero `purge_after_minutes` to exercise the purge path.
pub fn sweeper_with_session_config(
    agents: Vec<String>,
    session_config: PerAgentSessionConfig,
) -> (Arc<MemStorage>, ArchiveSweeper) {
    let mem = Arc::new(MemStorage::default());
    let config = Arc::new(MockConfig::with_agents(agents));
    *config.session_config.lock().unwrap() = session_config;
    let storage: Arc<dyn PersistenceService> = Arc::clone(&mem) as _;
    let sweeper = ArchiveSweeper::new(storage, config);
    (mem, sweeper)
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

/// Mock TaskManager that records `cleanup_all_finished` calls.
///
/// Callers hold the mock directly (e.g. `Arc::new(MockTaskManager::new())`)
/// and read the recorded state back via [`Self::was_called`] /
/// [`Self::last_session_id`].
#[derive(Default)]
pub struct MockTaskManager {
    cleanup_all_finished_called: Mutex<bool>,
    last_session_id: Mutex<Option<String>>,
}

impl MockTaskManager {
    /// Create a mock with no recorded calls yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `cleanup_all_finished` has been invoked.
    pub fn was_called(&self) -> bool {
        *self.cleanup_all_finished_called.lock().unwrap()
    }

    /// `session_id` passed to the most recent `cleanup_all_finished` call.
    pub fn last_session_id(&self) -> Option<String> {
        self.last_session_id.lock().unwrap().clone()
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
        *self.last_session_id.lock().unwrap() = Some(session_id.to_owned());
    }
    fn max_execution_secs(&self) -> u64 {
        3600
    }
}
