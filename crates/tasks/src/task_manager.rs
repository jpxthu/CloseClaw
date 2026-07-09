//! Trait for managing background tasks.
//!
//! Provides an interface for spawning, monitoring, and killing
//! background processes. Implemented by [`BackgroundTaskManager`].

use crate::{BackgroundTask, BackgroundTaskError, CompletionNotification};

/// Trait for managing background tasks.
///
/// Implemented by [`BackgroundTaskManager`](crate::BackgroundTaskManager);
/// consumed by the tools crate's `BashTool` to spawn and manage
/// background processes.
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

    /// Drain all pending completion notifications.
    async fn drain_notifications(&self) -> Vec<CompletionNotification>;

    /// Remove output files and handles for tasks that have reached
    /// a terminal state (Completed, Failed, Killed).
    async fn cleanup_finished(&self);
}
