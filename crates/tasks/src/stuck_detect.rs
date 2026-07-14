//! Stuck detection for background tasks.
//!
//! Monitors output file growth and detects interactive prompts
//! that indicate a task is waiting for user input.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::background::{CompletionNotification, NotificationPriority, TaskMap, TaskState};

/// Check interval between stuck detection polls.
const CHECK_INTERVAL_SECS: u64 = 5;
/// Seconds of output stall before declaring a task stuck.
const STUCK_TIMEOUT_SECS: u64 = 45;
/// Maximum bytes read from the tail of the output file.
const TAIL_READ_BYTES: usize = 4096;

/// Patterns indicating an interactive prompt in output.
const INTERACTIVE_PATTERNS: &[&str] = &[
    "(y/n)",
    "[Y/n]",
    "[y/N]",
    "Continue?",
    "Are you sure?",
    "Password:",
    "password:",
];

/// Configuration for the stuck detection monitor.
#[derive(Debug, Clone)]
pub(crate) struct StuckDetectConfig {
    pub(crate) check_interval: Duration,
    pub(crate) stuck_timeout: Duration,
}

impl Default for StuckDetectConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(CHECK_INTERVAL_SECS),
            stuck_timeout: Duration::from_secs(STUCK_TIMEOUT_SECS),
        }
    }
}

/// Start monitoring a running task for stuck conditions.
pub(crate) fn start_stuck_detection(
    task_id: String,
    output_path: PathBuf,
    command: String,
    tasks: TaskMap,
    notifications: Arc<Mutex<Vec<CompletionNotification>>>,
    config: StuckDetectConfig,
) {
    tokio::spawn(async move {
        monitor_loop(task_id, output_path, command, tasks, notifications, config).await;
    });
}

async fn monitor_loop(
    task_id: String,
    output_path: PathBuf,
    command: String,
    tasks: TaskMap,
    notifications: Arc<Mutex<Vec<CompletionNotification>>>,
    config: StuckDetectConfig,
) {
    let mut last_size: u64 = 0;
    let mut last_growth = tokio::time::Instant::now();
    let mut interval = tokio::time::interval(config.check_interval);
    interval.tick().await;

    loop {
        interval.tick().await;

        if !is_task_running(&tasks, &task_id).await {
            return;
        }

        let size = match get_output_size(&output_path).await {
            Some(s) => s,
            None => continue,
        };

        if size > last_size {
            last_size = size;
            last_growth = tokio::time::Instant::now();
        }

        if last_growth.elapsed() < config.stuck_timeout {
            continue;
        }

        let tail = read_tail(&output_path).await;
        if !has_interactive_prompt(&tail) {
            continue;
        }

        emit_stuck_alert(&tasks, &notifications, &task_id, &command, &output_path).await;
        return;
    }
}

async fn is_task_running(tasks: &TaskMap, task_id: &str) -> bool {
    let map = tasks.lock().await;
    matches!(
        map.get(task_id),
        Some(h) if matches!(h.state, TaskState::Running { .. })
    )
}

async fn get_output_size(path: &Path) -> Option<u64> {
    tokio::fs::metadata(path).await.ok().map(|m| m.len())
}

async fn emit_stuck_alert(
    tasks: &TaskMap,
    notifications: &Arc<Mutex<Vec<CompletionNotification>>>,
    task_id: &str,
    command: &str,
    output_path: &Path,
) {
    let mut map = tasks.lock().await;
    if let Some(h) = map.get_mut(task_id) {
        if h.notified {
            return;
        }
        h.notified = true;
        let summary = format!(
            "Background command '{}' appears stuck at an interactive prompt",
            command
        );
        let notif = CompletionNotification {
            task_id: h.id.clone(),
            command: command.to_owned(),
            state: h.state.clone(),
            output_path: output_path.to_path_buf(),
            priority: NotificationPriority::Next,
            summary,
        };
        notifications.lock().await.push(notif);
        tracing::warn!(
            task_id = %task_id,
            command = %h.command,
            "task stuck at interactive prompt"
        );
    }
}

async fn read_tail(path: &Path) -> String {
    let data = match tokio::fs::read(path).await {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let start = data.len().saturating_sub(TAIL_READ_BYTES);
    String::from_utf8_lossy(&data[start..]).to_string()
}

fn has_interactive_prompt(tail: &str) -> bool {
    INTERACTIVE_PATTERNS.iter().any(|p| tail.contains(p))
}

#[cfg(test)]
#[path = "stuck_detect_tests.rs"]
mod tests;
