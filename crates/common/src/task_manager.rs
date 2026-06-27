//! Task manager trait and types for decoupling tools from the tasks module.
//!
//! Provides an interface for managing background tasks without requiring
//! a direct dependency on the concrete `BackgroundTaskManager`.

use std::path::PathBuf;

/// Lifecycle state of a background task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    /// Process is running.
    Running,
    /// Process exited successfully.
    Completed { exit_code: i32 },
    /// Process exited with a non-zero exit code.
    Failed { exit_code: i32 },
    /// Process was killed externally.
    Killed,
}

/// A handle to a background task.
#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub id: String,
    pub command: String,
    pub state: TaskState,
    pub output_path: PathBuf,
}

/// Errors returned by the task manager.
#[derive(Debug, thiserror::Error)]
pub enum BackgroundTaskError {
    #[error("task not found: {0}")]
    NotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("task already completed")]
    AlreadyCompleted,
}

/// Trait for managing background tasks.
///
/// Implemented by `BackgroundTaskManager` in the main crate; used by the
/// tools crate's `BashTool` to spawn and manage background processes.
#[async_trait::async_trait]
pub trait TaskManager: Send + Sync {
    /// Spawn a shell command in the background, returning immediately.
    async fn spawn_task(
        &self,
        command: &str,
        cwd: &std::path::Path,
    ) -> Result<BackgroundTask, BackgroundTaskError>;

    /// Take over a running child process and manage it in the background.
    async fn backgroundize_task(
        &self,
        child: tokio::process::Child,
        command: &str,
    ) -> Result<BackgroundTask, BackgroundTaskError>;

    /// Kill a running background task by ID.
    async fn kill_task(&self, task_id: &str) -> Result<(), BackgroundTaskError>;

    /// Get a background task by ID.
    async fn get_task(&self, task_id: &str) -> Option<BackgroundTask>;
}
