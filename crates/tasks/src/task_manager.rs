//! Trait for managing background tasks.
//!
//! Provides an interface for spawning, monitoring, and killing
//! background processes. Implemented by [`BackgroundTaskManager`].

use crate::{BackgroundTask, BackgroundTaskError, CompletionNotification};

/// Lightweight summary of a currently running background task.
///
/// Returned by [`TaskManager::list_running_tasks`] so the caller
/// can inject a running-task digest without querying individual tasks.
#[derive(Debug, Clone)]
pub struct RunningTaskInfo {
    /// Unique identifier of the background task.
    pub task_id: String,
    /// The original shell command.
    pub command: String,
    /// Seconds elapsed since the task was created.
    pub elapsed_secs: u64,
}

/// Trait for managing background tasks.
///
/// Implemented by [`BackgroundTaskManager`](crate::BackgroundTaskManager);
/// consumed by the tools crate's `BashTool` to spawn and manage
/// background processes.
#[async_trait::async_trait]
pub trait TaskManager: Send + Sync {
    /// Spawn a shell command in the background, returning immediately.
    ///
    /// When `is_backgrounded` is `true`, the task was created via
    /// auto-backgrounding or manual backgrounding (user-initiated);
    /// `false` means explicit `run_in_background`.
    async fn spawn_task(
        &self,
        command: &str,
        cwd: &std::path::Path,
        is_backgrounded: bool,
    ) -> Result<BackgroundTask, BackgroundTaskError>;

    /// Take over a running child process and manage it in the background.
    ///
    /// When `is_backgrounded` is `true`, the task was created via
    /// auto-backgrounding or manual backgrounding (user-initiated);
    /// `false` means explicit `run_in_background`.
    async fn backgroundize_task(
        &self,
        child: tokio::process::Child,
        command: &str,
        is_backgrounded: bool,
    ) -> Result<BackgroundTask, BackgroundTaskError>;

    /// Kill a running background task by ID.
    async fn kill_task(&self, task_id: &str) -> Result<(), BackgroundTaskError>;

    /// Get a background task by ID.
    async fn get_task(&self, task_id: &str) -> Option<BackgroundTask>;

    /// List all currently running background tasks.
    ///
    /// Returns a snapshot of tasks in the [`TaskState::Running`] state,
    /// each summarised as a [`RunningTaskInfo`].
    async fn list_running_tasks(&self) -> Vec<RunningTaskInfo>;

    /// Drain all pending completion notifications.
    async fn drain_notifications(&self) -> Vec<CompletionNotification>;

    /// Remove output files and handles for tasks that have reached
    /// a terminal state (Completed, Failed, Killed).
    async fn cleanup_finished(&self);
}
