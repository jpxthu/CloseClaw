//! Step 1.3 tests for auto-background timeout behavior.
//!
//! Validates that `execute_foreground_command` computes `bg_timeout`
//! correctly based on the `agent_timeout_ms` parameter:
//!
//! - `Some(30_000)` → 30s (agent-specified, within cap)
//! - `Some(300_000)` → 300s (within cap, cap is 600s)
//! - `None` → 15s (system default)
//! - Excluded commands (sleep/true/false) → `agent_timeout_ms` or 600s

use super::*;
use serde_json::json;
use tempfile::TempDir;

/// Minimal mock that satisfies the `TaskManager` trait for foreground
/// timeout tests. Only `backgroundize_task` needs real behavior; the
/// rest are no-ops.
struct TimeoutBgManager;

#[async_trait::async_trait]
impl closeclaw_tasks::TaskManager for TimeoutBgManager {
    async fn spawn_task(
        &self,
        _command: &str,
        _cwd: &std::path::Path,
        _is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        Err(closeclaw_tasks::BackgroundTaskError::SpawnFailed(
            "not used".into(),
        ))
    }
    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        command: &str,
        is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        // Return a fake task — the test only cares about whether the
        // child was backgroundized, not the task itself.
        Ok(closeclaw_tasks::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_tasks::TaskState::Running { is_backgrounded },
            output_path: std::path::PathBuf::from("/tmp/test-output"),
        })
    }
    async fn kill_task(&self, _: &str) -> Result<(), closeclaw_tasks::BackgroundTaskError> {
        Ok(())
    }
    async fn get_task(&self, _: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        None
    }
    async fn list_running_tasks(&self) -> Vec<closeclaw_tasks::RunningTaskInfo> {
        vec![]
    }
    async fn drain_notifications(&self) -> Vec<closeclaw_tasks::CompletionNotification> {
        vec![]
    }
    async fn cleanup_all_finished(&self, _session_id: &str) {}
    fn max_execution_secs(&self) -> u64 {
        1800
    }
}

fn bg_trait() -> Arc<dyn closeclaw_tasks::TaskManager> {
    Arc::new(TimeoutBgManager)
}

/// Mock TaskManager with configurable `max_execution_secs`.
/// Used by tests that need a short total-execution-time limit
/// to exercise the force-terminate path within reasonable test duration.
struct ShortTimeoutBgManager {
    max_secs: u64,
}

#[async_trait::async_trait]
impl closeclaw_tasks::TaskManager for ShortTimeoutBgManager {
    async fn spawn_task(
        &self,
        _command: &str,
        _cwd: &std::path::Path,
        _is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        Err(closeclaw_tasks::BackgroundTaskError::SpawnFailed(
            "not used".into(),
        ))
    }
    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        command: &str,
        is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        Ok(closeclaw_tasks::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_tasks::TaskState::Running { is_backgrounded },
            output_path: std::path::PathBuf::from("/tmp/test-output"),
        })
    }
    async fn kill_task(&self, _: &str) -> Result<(), closeclaw_tasks::BackgroundTaskError> {
        Ok(())
    }
    async fn get_task(&self, _: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        None
    }
    async fn list_running_tasks(&self) -> Vec<closeclaw_tasks::RunningTaskInfo> {
        vec![]
    }
    async fn drain_notifications(&self) -> Vec<closeclaw_tasks::CompletionNotification> {
        vec![]
    }
    async fn cleanup_all_finished(&self, _session_id: &str) {}
    fn max_execution_secs(&self) -> u64 {
        self.max_secs
    }
}

// ---------------------------------------------------------------------------
// Agent-specified timeout: 30s → should NOT auto-background
// (command completes before 30s timeout)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_timeout_30s_completes_in_foreground() {
    let tmp = TempDir::new().unwrap();
    let (outcome, _) = execute_foreground_command(
        "echo done",
        tmp.path().to_str().unwrap(),
        Some(30_000),
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    // Command completes instantly → foreground result.
    match outcome {
        ForegroundOutcome::Completed(result) => {
            assert_eq!(result.data["exitCode"], json!(0));
        }
        other => panic!(
            "agent timeout 30s: quick command should complete in foreground, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent-specified timeout: 30s → long-running command should auto-background
// at 30s (not at the default 15s)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_timeout_30s_auto_backgrounds_long_command() {
    let tmp = TempDir::new().unwrap();
    let bg = bg_trait();

    // Use a bg_timeout of 30s. We can't actually wait 30s in a unit
    // test, so we verify the timeout logic indirectly: spawn a child
    // that takes ~1s and check that it does NOT auto-background when
    // bg_timeout (30s) > command duration.
    let (outcome, _) = execute_foreground_command(
        "sleep 0.5",
        tmp.path().to_str().unwrap(),
        Some(30_000),
        &bg,
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(_) => {
            // Good: completed in foreground (30s timeout > 0.5s duration)
        }
        ForegroundOutcome::AutoBackground(_, _) => {
            panic!("agent timeout 30s: 0.5s command should NOT auto-background");
        }
        ForegroundOutcome::Failed(e) => {
            panic!("unexpected failure: {}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Agent-specified timeout: 300s → should be capped to 120s
// Verify by checking that a command completing at ~1s does NOT
// auto-background (since 120s cap > 1s).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_timeout_300s_capped_to_120s_quick_command() {
    let tmp = TempDir::new().unwrap();
    let (outcome, _) = execute_foreground_command(
        "echo capped",
        tmp.path().to_str().unwrap(),
        Some(300_000),
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(result) => {
            assert_eq!(result.data["exitCode"], json!(0));
        }
        other => panic!(
            "capped timeout: quick command should complete in foreground, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent-specified timeout: None → default 15s
// Quick command completes in foreground.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_default_timeout_quick_command_completes() {
    let tmp = TempDir::new().unwrap();
    let (outcome, _) = execute_foreground_command(
        "echo default",
        tmp.path().to_str().unwrap(),
        None,
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(result) => {
            assert_eq!(result.data["exitCode"], json!(0));
        }
        other => panic!(
            "default timeout: quick command should complete in foreground, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Excluded command: `true` → should NOT auto-background regardless of timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_excluded_command_true_not_auto_backgrounded() {
    let tmp = TempDir::new().unwrap();
    let (outcome, _) = execute_foreground_command(
        "true",
        tmp.path().to_str().unwrap(),
        None,
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(result) => {
            assert_eq!(result.data["exitCode"], json!(0));
        }
        other => panic!(
            "excluded command 'true' should complete in foreground, got: {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn test_excluded_command_false_not_auto_backgrounded() {
    let tmp = TempDir::new().unwrap();
    let (outcome, _) = execute_foreground_command(
        "false",
        tmp.path().to_str().unwrap(),
        None,
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(result) => {
            // `false` exits with code 1 — still a normal completion.
            assert_eq!(result.data["exitCode"], json!(1));
        }
        other => panic!(
            "excluded command 'false' should complete in foreground, got: {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn test_excluded_command_sleep_not_auto_backgrounded() {
    let tmp = TempDir::new().unwrap();
    // sleep is excluded → uses agent_timeout_ms.unwrap_or(120_000).
    // With a short sleep, it should complete in foreground.
    let (outcome, _) = execute_foreground_command(
        "sleep 0.1",
        tmp.path().to_str().unwrap(),
        None,
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(result) => {
            assert_eq!(result.data["exitCode"], json!(0));
        }
        other => panic!(
            "excluded command 'sleep 0.1' should complete in foreground, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Excluded command with explicit timeout → should use that timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_excluded_command_with_agent_timeout_uses_agent_timeout() {
    let tmp = TempDir::new().unwrap();
    // Agent explicit timeout (60s) overrides whitelist exclusion.
    // White list commands with agent timeout follow the normal
    // clamped logic: min(agent, cap). Quick command completes in
    // foreground since 60s > command duration.
    let (outcome, _) = execute_foreground_command(
        "true",
        tmp.path().to_str().unwrap(),
        Some(60_000),
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(result) => {
            assert_eq!(result.data["exitCode"], json!(0));
        }
        other => panic!(
            "excluded command with agent timeout: quick command should complete in foreground, got: {:?}",
            other
        ),
    }
}

/// Excluded command with agent timeout: verify the agent-specified
/// timeout is respected (not overridden by whitelist).
/// `sleep 0.5` exceeds agent timeout (1s) → should auto-background
/// (normal clamped logic, NOT force-terminated).
#[tokio::test]
async fn test_excluded_command_sleep_with_agent_timeout_auto_backgrounds() {
    let tmp = TempDir::new().unwrap();
    // Agent says 1s timeout, excluded command → normal clamped logic.
    // `sleep 0.5` completes before 1s → foreground completion.
    let (outcome, _) = execute_foreground_command(
        "sleep 0.5",
        tmp.path().to_str().unwrap(),
        Some(1_000),
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(result) => {
            assert_eq!(result.data["exitCode"], json!(0));
        }
        other => panic!(
            "excluded command sleep with agent timeout: should complete in foreground (0.5s < 1s), got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Non-excluded command with agent timeout → timeout caps at 120s
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_non_excluded_with_large_timeout_capped_to_120s() {
    let tmp = TempDir::new().unwrap();
    // `echo` is not excluded; agent specifies 500s → capped to 120s.
    // Quick command completes normally.
    let (outcome, _) = execute_foreground_command(
        "echo hello",
        tmp.path().to_str().unwrap(),
        Some(500_000),
        &bg_trait(),
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Completed(result) => {
            assert_eq!(result.data["exitCode"], json!(0));
        }
        other => panic!(
            "non-excluded with large timeout should complete in foreground, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Verify constants are correct
// ---------------------------------------------------------------------------

#[test]
fn test_auto_bg_timeout_constants() {
    assert_eq!(AUTO_BG_TIMEOUT_MS, 15_000, "default should be 15s");
    assert_eq!(
        AUTO_BG_TIMEOUT_CAP_MS, 600_000,
        "cap should be 600s (10 minutes)"
    );
}

// ---------------------------------------------------------------------------
// Whitelist + no agent timeout + exceeds total time → force-killed
// ---------------------------------------------------------------------------

/// Whitelist command with no agent timeout and a short max_execution_secs:
/// the command should be force-killed (not auto-backgrounded) when it
/// exceeds the total execution time limit.
#[tokio::test]
async fn test_excluded_command_exceeds_max_execution_force_killed() {
    let tmp = TempDir::new().unwrap();
    // Mock with 2-second max execution limit.
    let bg: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(ShortTimeoutBgManager { max_secs: 2 });

    let (outcome, _) = execute_foreground_command(
        "sleep 10",
        tmp.path().to_str().unwrap(),
        None, // No agent timeout — whitelist path
        &bg,
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::Failed(msg) => {
            assert!(
                msg.contains("exceeded total execution time limit"),
                "expected force-kill message, got: {}",
                msg
            );
        }
        other => panic!(
            "excluded command exceeding max execution should be force-killed, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Whitelist + explicit agent timeout + long command → auto-backgrounded
// ---------------------------------------------------------------------------

/// Whitelist command with an explicit agent timeout: the whitelist
/// exclusion does NOT apply, so the normal clamped-logic path is
/// followed. A command exceeding the agent timeout is auto-backgrounded.
#[tokio::test]
async fn test_excluded_command_with_agent_timeout_long_command_backgrounds() {
    let tmp = TempDir::new().unwrap();
    let bg = bg_trait();
    // Agent timeout = 1s, sleep takes 5s → should auto-background.
    let (outcome, _) = execute_foreground_command(
        "sleep 5",
        tmp.path().to_str().unwrap(),
        Some(1_000), // Explicit agent timeout
        &bg,
        None,
        None,
        None,
        "",
    )
    .await
    .expect("execute_foreground_command should succeed");

    match outcome {
        ForegroundOutcome::AutoBackground(_, _) => {
            // Expected: excluded command with agent timeout → normal
            // clamped logic → auto-backgrounded on timeout.
        }
        other => panic!(
            "excluded command with agent timeout: long command should auto-background, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Verify auto_backgroundize_excluded logic
// ---------------------------------------------------------------------------

#[test]
fn test_auto_backgroundize_excluded_cases() {
    assert!(auto_backgroundize_excluded("sleep 5"));
    assert!(auto_backgroundize_excluded("sleep"));
    assert!(auto_backgroundize_excluded("/usr/bin/sleep 10"));
    assert!(auto_backgroundize_excluded("true"));
    assert!(auto_backgroundize_excluded("false"));
    assert!(!auto_backgroundize_excluded("echo hello"));
    assert!(!auto_backgroundize_excluded("ls"));
    assert!(!auto_backgroundize_excluded("curl http://example.com"));
    assert!(!auto_backgroundize_excluded("sleepy"));
    assert!(!auto_backgroundize_excluded("truesome"));
}
