//! Integration tests for debug_log emission in BackgroundTaskManager.
//!
//! These tests verify that when a `DebugLog` is provided via
//! `with_debug_log()`, structured events are emitted for task
//! lifecycle transitions (started → terminal).
//!
//! Compiled via `#[path = "debug_log_integration_tests.rs"]` inside
//! `background.rs`'s `#[cfg(test)] mod debug_log_integration_tests`.

use super::*;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

use closeclaw_debug_log::test_utils::{make_debug_log_at_level, read_events};
use closeclaw_debug_log::LogLevel;

// ── helpers ──────────────────────────────────────────────────────────────

async fn make_manager_with_debug(dir: &TempDir) -> BackgroundTaskManager {
    let dlog = make_debug_log_at_level(dir, LogLevel::Debug).await;
    BackgroundTaskManager::with_temp_dir(dir.path()).with_debug_log(Arc::new(dlog))
}

async fn wait_for_terminal(mgr: &BackgroundTaskManager, task_id: &str) -> BackgroundTask {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(s) = mgr.get_task(task_id).await {
                if !matches!(s.state, TaskState::Running { .. }) {
                    return s;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("task did not reach terminal state within timeout")
}

// ── 1. with_debug_log: started event emitted ─────────────────────────────

/// When a `DebugLog` is provided, spawning a task emits a
/// `background.task.started` event with task_id and command in payload.
#[tokio::test]
async fn test_debug_log_started_event() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager_with_debug(&tmp).await;
    let task = mgr
        .spawn("echo hello", tmp.path(), false, "test-session")
        .await
        .unwrap();
    // Give the spawned log task time to write.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events = read_events(tmp.path()).await;
    let started: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "background.task.started")
        .collect();
    assert_eq!(
        started.len(),
        1,
        "expected exactly one started event, got {}",
        started.len()
    );

    let ev = started[0];
    assert_eq!(ev.level, LogLevel::Info);
    assert_eq!(ev.source_module, "tasks");
    assert!(!ev.trace_id.is_empty(), "trace_id must not be empty");
    assert!(
        ev.trace_id.starts_with("tasks-"),
        "trace_id should start with 'tasks-', got: {}",
        ev.trace_id
    );
    assert_eq!(ev.payload["task_id"], task.id);
    assert_eq!(ev.payload["command"], "echo hello");

    // Cleanup.
    let _ = mgr.kill(&task.id).await;
}

// ── 2. with_debug_log: terminal event on completion ──────────────────────

/// A task that completes successfully emits a `background.task.terminal`
/// event with `new_state: "completed"`.
#[tokio::test]
async fn test_debug_log_terminal_completed_event() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager_with_debug(&tmp).await;
    let task = mgr
        .spawn("true", tmp.path(), false, "test-session")
        .await
        .unwrap();
    wait_for_terminal(&mgr, &task.id).await;
    // Give spawned log tasks time to finish writing.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events = read_events(tmp.path()).await;
    let terminal: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "background.task.terminal")
        .collect();
    assert!(!terminal.is_empty(), "expected at least one terminal event");

    let ev = terminal[0];
    assert_eq!(ev.level, LogLevel::Info);
    assert_eq!(ev.source_module, "tasks");
    assert!(!ev.trace_id.is_empty());
    assert_eq!(ev.payload["task_id"], task.id);
    assert_eq!(ev.payload["command"], "true");
    assert_eq!(ev.payload["new_state"], "completed");
}

// ── 3. with_debug_log: terminal event on failure ─────────────────────────

/// A task that fails emits a `background.task.terminal` event with
/// `new_state: "failed"`.
#[tokio::test]
async fn test_debug_log_terminal_failed_event() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager_with_debug(&tmp).await;
    let task = mgr
        .spawn("false", tmp.path(), false, "test-session")
        .await
        .unwrap();
    wait_for_terminal(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events = read_events(tmp.path()).await;
    let terminal: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "background.task.terminal")
        .collect();
    assert!(!terminal.is_empty());

    let ev = terminal[0];
    assert_eq!(ev.payload["new_state"], "failed");
    assert_eq!(ev.payload["task_id"], task.id);
}

// ── 4. with_debug_log: terminal event on kill ────────────────────────────

/// A task that is killed emits a `background.task.terminal` event with
/// `new_state: "killed"`.
#[tokio::test]
async fn test_debug_log_terminal_killed_event() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager_with_debug(&tmp).await;
    let task = mgr
        .spawn("sleep 60", tmp.path(), false, "test-session")
        .await
        .unwrap();
    // Wait a bit for the process to start.
    tokio::time::sleep(Duration::from_millis(200)).await;
    mgr.kill(&task.id).await.unwrap();
    wait_for_terminal(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events = read_events(tmp.path()).await;
    let terminal: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "background.task.terminal")
        .collect();
    assert!(!terminal.is_empty());

    let ev = terminal[0];
    assert_eq!(ev.payload["new_state"], "killed");
    assert_eq!(ev.payload["task_id"], task.id);
}

// ── 5. with_debug_log: trace_id shared between started and terminal ──────

/// The same trace_id must be used for both the started event and the
/// terminal event of the same task.
#[tokio::test]
async fn test_debug_log_trace_id_shared() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager_with_debug(&tmp).await;
    let task = mgr
        .spawn("true", tmp.path(), false, "test-session")
        .await
        .unwrap();
    wait_for_terminal(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events = read_events(tmp.path()).await;
    let task_events: Vec<_> = events
        .iter()
        .filter(|e| {
            e.event_type == "background.task.started" || e.event_type == "background.task.terminal"
        })
        .filter(|e| e.payload["task_id"] == task.id)
        .collect();

    assert!(
        task_events.len() >= 2,
        "expected started + terminal events, got {}",
        task_events.len()
    );

    let trace_ids: Vec<&str> = task_events.iter().map(|e| e.trace_id.as_str()).collect();
    let first = trace_ids[0];
    assert!(
        trace_ids.iter().all(|tid| *tid == first),
        "all events for the same task must share the same trace_id, got: {:?}",
        trace_ids
    );
}

// ── 6. with_debug_log: event count matches expectation ───────────────────

/// For a successful task, exactly 2 events (started + terminal) are
/// emitted.
#[tokio::test]
async fn test_debug_log_event_count() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager_with_debug(&tmp).await;
    let task = mgr
        .spawn("true", tmp.path(), false, "test-session")
        .await
        .unwrap();
    wait_for_terminal(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events = read_events(tmp.path()).await;
    let task_events: Vec<_> = events
        .iter()
        .filter(|e| e.payload["task_id"] == task.id)
        .collect();
    assert_eq!(
        task_events.len(),
        2,
        "successful task should have 2 events (started + terminal), got {}",
        task_events.len()
    );
    let types: Vec<&str> = task_events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(types.contains(&"background.task.started"));
    assert!(types.contains(&"background.task.terminal"));
}

// ── 7. no debug_log: no events emitted ───────────────────────────────────

/// When `debug_log` is `None` (default), no JSONL files are created.
#[tokio::test]
async fn test_no_debug_log_no_events() {
    let tmp = TempDir::new().unwrap();
    let mgr = BackgroundTaskManager::with_temp_dir(tmp.path());
    let task = mgr
        .spawn("true", tmp.path(), false, "test-session")
        .await
        .unwrap();
    wait_for_terminal(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut dir = tokio::fs::read_dir(tmp.path()).await.unwrap();
    let mut jsonl_count = 0;
    while let Ok(Some(entry)) = dir.next_entry().await {
        if entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl") {
            jsonl_count += 1;
        }
    }
    assert_eq!(
        jsonl_count, 0,
        "no JSONL files should exist when debug_log is None"
    );
}

// ── 8. debug_log with max_execution_secs kill: terminal event emitted ────

/// When the total-execution-time-limit monitor kills a task, a
/// `background.task.terminal` event with `new_state: "killed"` is emitted.
#[tokio::test]
async fn test_debug_log_max_execution_kill_emits_terminal() {
    let tmp = TempDir::new().unwrap();
    let dlog = make_debug_log_at_level(&tmp, LogLevel::Debug).await;
    let mgr = BackgroundTaskManager::with_max_execution_secs_unchecked(tmp.path(), 1)
        .with_debug_log(Arc::new(dlog));
    let task = mgr
        .spawn("sleep 60", tmp.path(), false, "test-session")
        .await
        .unwrap();
    wait_for_terminal(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events = read_events(tmp.path()).await;
    let terminal: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "background.task.terminal")
        .collect();
    assert!(
        !terminal.is_empty(),
        "timeout-killed task should emit terminal event"
    );
    assert_eq!(terminal[0].payload["new_state"], "killed");
}
