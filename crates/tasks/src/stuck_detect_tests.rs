//! Unit tests for stuck detection.
//!
//! These tests are compiled via `#[path = "stuck_detect_tests.rs"]` inside
//! `stuck_detect.rs`'s `#[cfg(test)] mod tests`.

use super::*;
use crate::background::{CompletionNotification, TaskHandle, TaskMap, TaskState};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// has_interactive_prompt — pattern matching
// ---------------------------------------------------------------------------

#[test]
fn test_has_interactive_prompt_y_n() {
    assert!(has_interactive_prompt("Do you want to continue? (y/n) "));
}

#[test]
fn test_has_interactive_prompt_bracket_y_n() {
    assert!(has_interactive_prompt("Proceed? [Y/n] "));
}

#[test]
fn test_has_interactive_prompt_bracket_y_capital_n() {
    assert!(has_interactive_prompt("[y/N] "));
}

#[test]
fn test_has_interactive_prompt_continue() {
    assert!(has_interactive_prompt("Continue? "));
}

#[test]
fn test_has_interactive_prompt_are_you_sure() {
    assert!(has_interactive_prompt("Are you sure? "));
}

#[test]
fn test_has_interactive_prompt_password() {
    assert!(has_interactive_prompt("Password: "));
}

#[test]
fn test_has_interactive_prompt_lowercase_password() {
    assert!(has_interactive_prompt("password: "));
}

#[test]
fn test_has_interactive_prompt_no_match() {
    assert!(!has_interactive_prompt(
        "Normal output line\nAnother line\n"
    ));
}

#[test]
fn test_has_interactive_prompt_empty() {
    assert!(!has_interactive_prompt(""));
}

#[test]
fn test_has_interactive_prompt_partial_word() {
    // "password" without colon does not match "Password:"
    assert!(!has_interactive_prompt("The password field is required"));
}

#[test]
fn test_has_interactive_prompt_embedded_in_line() {
    assert!(has_interactive_prompt(
        "Package configured. Continue? (y/n) "
    ));
}

// ---------------------------------------------------------------------------
// stuck detection flow — triggers with short threshold
// ---------------------------------------------------------------------------

/// Insert a Running task handle into the task map.
async fn create_test_task(tasks: &TaskMap, task_id: &str, output_path: &std::path::Path) {
    let mut map = tasks.lock().await;
    map.insert(
        task_id.to_string(),
        TaskHandle {
            id: task_id.to_string(),
            command: "test command".to_string(),
            state: TaskState::Running {
                is_backgrounded: false,
            },
            output_path: output_path.to_path_buf(),
            kill_tx: None,
            notified: false,
            is_backgrounded: false,
        },
    );
}

#[tokio::test]
async fn test_stuck_detection_triggers() {
    let tmp = TempDir::new().unwrap();
    let output_path = tmp.path().join("output");
    let task_id = "stuck-trigger-test";

    // Write output containing an interactive prompt
    tokio::fs::write(&output_path, "Installing packages...\nContinue? (y/n) ")
        .await
        .unwrap();

    let tasks: TaskMap = Arc::new(Mutex::new(HashMap::new()));
    let notifications: Arc<Mutex<Vec<CompletionNotification>>> = Arc::new(Mutex::new(Vec::new()));

    create_test_task(&tasks, task_id, &output_path).await;

    let config = StuckDetectConfig {
        check_interval: Duration::from_secs(1),
        stuck_timeout: Duration::from_secs(1),
    };

    start_stuck_detection(
        task_id.to_string(),
        output_path,
        "test command".to_string(),
        tasks,
        notifications.clone(),
        config,
    );

    // Wait: 1s interval + 1s timeout + margin for task scheduling on busy systems
    tokio::time::sleep(Duration::from_secs(5)).await;

    let notifs = notifications.lock().await;
    assert!(
        !notifs.is_empty(),
        "expected a stuck detection notification"
    );
    assert_eq!(notifs[0].priority, NotificationPriority::Next);
    assert_eq!(notifs[0].task_id, task_id);
}

// ---------------------------------------------------------------------------
// stuck detection flow — no trigger for growing output
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_stuck_detection_no_trigger_growing_output() {
    let tmp = TempDir::new().unwrap();
    let output_path = tmp.path().join("output");
    let task_id = "stuck-no-trigger-test";

    // Start with initial content
    tokio::fs::write(&output_path, "step 0\n").await.unwrap();

    let tasks: TaskMap = Arc::new(Mutex::new(HashMap::new()));
    let notifications: Arc<Mutex<Vec<CompletionNotification>>> = Arc::new(Mutex::new(Vec::new()));

    create_test_task(&tasks, task_id, &output_path).await;

    let config = StuckDetectConfig {
        check_interval: Duration::from_secs(1),
        stuck_timeout: Duration::from_secs(2),
    };

    start_stuck_detection(
        task_id.to_string(),
        output_path.clone(),
        "test command".to_string(),
        tasks,
        notifications.clone(),
        config,
    );

    // Append growing content to keep resetting the stuck timer
    for i in 1..=5 {
        tokio::time::sleep(Duration::from_millis(800)).await;
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&output_path)
            .await
            .unwrap();
        file.write_all(format!("step {}\n", i).as_bytes())
            .await
            .unwrap();
    }

    let notifs = notifications.lock().await;
    assert!(
        notifs.is_empty(),
        "no stuck alert expected for continuously growing output"
    );
}

// ---------------------------------------------------------------------------
// stuck detection — skips non-running tasks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_stuck_detection_skips_non_running_task() {
    let tmp = TempDir::new().unwrap();
    let output_path = tmp.path().join("output");
    let task_id = "stuck-skip-test";

    tokio::fs::write(&output_path, "Continue? (y/n) ")
        .await
        .unwrap();

    let tasks: TaskMap = Arc::new(Mutex::new(HashMap::new()));
    let notifications: Arc<Mutex<Vec<CompletionNotification>>> = Arc::new(Mutex::new(Vec::new()));

    // Insert a task that is already Completed (not Running)
    {
        let mut map = tasks.lock().await;
        map.insert(
            task_id.to_string(),
            TaskHandle {
                id: task_id.to_string(),
                command: "test command".to_string(),
                state: TaskState::Completed { exit_code: 0 },
                output_path: output_path.clone(),
                kill_tx: None,
                notified: false,
                is_backgrounded: false,
            },
        );
    }

    let config = StuckDetectConfig {
        check_interval: Duration::from_secs(1),
        stuck_timeout: Duration::from_secs(1),
    };

    start_stuck_detection(
        task_id.to_string(),
        output_path,
        "test command".to_string(),
        tasks,
        notifications.clone(),
        config,
    );

    tokio::time::sleep(Duration::from_secs(3)).await;

    let notifs = notifications.lock().await;
    assert!(notifs.is_empty(), "no alert expected for non-running task");
}
