//! Unit tests for DaemonRunner trait and handle_run / handle_run_foreground.
//!
//! Covers the four behavioral dimensions required by the plan:
//! 1. handle_run_foreground calls DaemonRunner::start_and_run via mock
//! 2. handle_run(background) does NOT call DaemonRunner — spawns subprocess
//! 3. DaemonRunner error propagates through handle_run_foreground
//! 4. Foreground mode writes the PID file correctly

use super::run::{handle_run, handle_run_foreground, DaemonRunner};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

// ── Mock DaemonRunner ──────────────────────────────────────────────────────

/// Mock that records calls and can be configured to succeed or fail.
struct MockDaemonRunner {
    /// Set to `true` by `start_and_run` when invoked.
    called: Arc<AtomicBool>,
    /// If non-None, `start_and_run` returns this error.
    fail_msg: Option<String>,
}

impl MockDaemonRunner {
    fn success() -> Self {
        Self {
            called: Arc::new(AtomicBool::new(false)),
            fail_msg: None,
        }
    }

    fn failing(msg: impl Into<String>) -> Self {
        Self {
            called: Arc::new(AtomicBool::new(false)),
            fail_msg: Some(msg.into()),
        }
    }

    fn was_called(&self) -> bool {
        self.called.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl DaemonRunner for MockDaemonRunner {
    async fn start_and_run(&self, _config_dir: &str) -> anyhow::Result<()> {
        self.called.store(true, Ordering::SeqCst);
        if let Some(msg) = &self.fail_msg {
            anyhow::bail!("{}", msg);
        }
        Ok(())
    }
}

// ── Test 1: handle_run_foreground calls DaemonRunner ────────────────────────

/// handle_run_foreground must invoke DaemonRunner::start_and_run exactly once
/// and must NOT spawn a subprocess.
#[tokio::test]
async fn test_handle_run_foreground_calls_daemon_runner() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let mock = MockDaemonRunner::success();

    let result = handle_run_foreground(&config_dir, false, &mock).await;
    assert!(
        result.is_ok(),
        "handle_run_foreground should succeed: {result:?}"
    );
    assert!(
        mock.was_called(),
        "DaemonRunner::start_and_run should be called once"
    );
}

// ── Test 2: handle_run(background) does NOT call DaemonRunner ───────────────

/// When foreground=false, handle_run spawns a subprocess and must NOT call
/// DaemonRunner::start_and_run.
#[tokio::test]
async fn test_handle_run_background_does_not_call_daemon_runner() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let mock = MockDaemonRunner::success();

    // foreground=false → subprocess spawn path. The spawn will fail because
    // the binary doesn't exist, but the key assertion is that the mock was
    // never called.
    let result = handle_run(config_dir, false, false, &mock).await;
    assert!(result.is_err(), "spawn should fail in test env");
    assert!(
        !mock.was_called(),
        "DaemonRunner::start_and_run must NOT be called in background mode"
    );
}

// ── Test 3: DaemonRunner error propagates ───────────────────────────────────

/// When DaemonRunner::start_and_run returns an error, handle_run_foreground
/// must propagate that error to the caller.
#[tokio::test]
async fn test_handle_run_foreground_propagates_daemon_runner_error() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let mock = MockDaemonRunner::failing("simulated daemon crash");

    let result = handle_run_foreground(&config_dir, false, &mock).await;
    assert!(result.is_err(), "should propagate the error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("simulated daemon crash"),
        "error message should contain the mock failure text, got: {err_msg}"
    );
}

// ── Test 4: PID file is written correctly in foreground mode ────────────────

/// In foreground mode, after the daemon runs, the PID file should contain the
/// current process's PID (written by handle_run_foreground before the daemon
/// runs). We verify the file exists and contains a valid PID.
#[tokio::test]
async fn test_handle_run_foreground_writes_pid_file() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    let mock = MockDaemonRunner::success();

    let result = handle_run_foreground(&config_dir, false, &mock).await;
    assert!(
        result.is_ok(),
        "handle_run_foreground should succeed: {result:?}"
    );
    assert!(mock.was_called(), "mock should have been called");

    // Verify PID file exists and contains a valid PID.
    let pid_file = closeclaw_platform::process::pid_file_path(tmp.path());
    assert!(
        pid_file.exists(),
        "PID file should exist at {}",
        pid_file.display()
    );
    let pid = closeclaw_platform::process::read_pid_file(&pid_file);
    assert!(pid.is_some(), "PID file should contain a parseable PID");
    // The PID should match the current process (written by handle_run_foreground).
    assert_eq!(
        pid.unwrap(),
        std::process::id(),
        "PID file should contain the current process ID"
    );
}
