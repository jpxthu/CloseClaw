//! Unit tests for DaemonRunner trait and handle_run / handle_run_foreground.
//!
//! Covers the four behavioral dimensions required by the plan:
//! 1. handle_run_foreground calls DaemonRunner::start_and_run via mock
//! 2. handle_run(background) does NOT call DaemonRunner — spawns subprocess
//! 3. DaemonRunner error propagates through handle_run_foreground
//! 4. Foreground mode writes the PID file correctly

use super::run::{
    ensure_no_running_daemon, handle_run, handle_run_foreground, prepare_run, DaemonRunner,
};
use closeclaw_platform::process::{pid_file_path, write_pid_file};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
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

// ── Test 5: Foreground rejects start when alive instance detected ───────────

/// When a PID file exists and the process is alive, handle_run_foreground
/// must bail with "daemon already running".
#[tokio::test]
async fn test_handle_run_foreground_rejects_alive_daemon() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    // Write our own PID to simulate a running daemon.
    let pid_file = closeclaw_platform::process::pid_file_path(tmp.path());
    closeclaw_platform::process::write_pid_file(&pid_file, std::process::id()).unwrap();

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock).await;
    assert!(result.is_err(), "should reject when daemon is alive");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("daemon already running"),
        "error should mention daemon already running, got: {err_msg}"
    );
    assert!(
        !mock.was_called(),
        "DaemonRunner should NOT be called when daemon is alive"
    );
}

// ── Test 6: Foreground cleans stale PID file and starts normally ────────────

/// When a PID file exists but the process is dead, handle_run_foreground
/// must clean the stale PID and proceed normally.
#[tokio::test]
async fn test_handle_run_foreground_cleans_stale_pid() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    // Write a PID that does not exist (stale).
    let pid_file = closeclaw_platform::process::pid_file_path(tmp.path());
    closeclaw_platform::process::write_pid_file(&pid_file, 99999999).unwrap();

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock).await;
    assert!(
        result.is_ok(),
        "should succeed after cleaning stale PID: {result:?}"
    );
    assert!(
        mock.was_called(),
        "DaemonRunner should be called after stale PID is cleaned"
    );
}

// ── Test 7: Foreground succeeds with no existing PID file ───────────────────

/// When no PID file exists, handle_run_foreground must proceed normally.
#[tokio::test]
async fn test_handle_run_foreground_no_pid_file() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock).await;
    assert!(
        result.is_ok(),
        "should succeed with no existing PID file: {result:?}"
    );
    assert!(mock.was_called(), "DaemonRunner should be called");
}

// ── Test 8: Background rejects start when alive instance detected ───────────

/// When a PID file exists and the process is alive, handle_run(background)
/// must bail with "daemon already running".
#[tokio::test]
async fn test_handle_run_background_rejects_alive_daemon() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    // Write our own PID to simulate a running daemon.
    let pid_file = closeclaw_platform::process::pid_file_path(tmp.path());
    closeclaw_platform::process::write_pid_file(&pid_file, std::process::id()).unwrap();

    let mock = MockDaemonRunner::success();
    let result = handle_run(config_dir, false, false, &mock).await;
    assert!(result.is_err(), "should reject when daemon is alive");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("daemon already running"),
        "error should mention daemon already running, got: {err_msg}"
    );
    assert!(
        !mock.was_called(),
        "DaemonRunner should NOT be called when daemon is alive"
    );
}

// ── Test 9: Background cleans stale PID file ────────────────────────────────

/// When a PID file exists but the process is dead, handle_run(background)
/// must clean the stale PID and proceed.
#[tokio::test]
async fn test_handle_run_background_cleans_stale_pid() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    // Write a PID that does not exist (stale).
    let pid_file = closeclaw_platform::process::pid_file_path(tmp.path());
    closeclaw_platform::process::write_pid_file(&pid_file, 99999999).unwrap();

    let mock = MockDaemonRunner::success();
    let _result = handle_run(config_dir, false, false, &mock).await;
    // It will fail because spawn fails (binary not found), but the key
    // point is that the stale PID was cleaned and it got past the check.
    assert!(
        !pid_file.exists(),
        "stale PID file should have been removed"
    );
    assert!(
        !mock.was_called(),
        "DaemonRunner should not be called in background mode"
    );
}

// ── ensure_no_running_daemon tests ──────────────────────────────────────

/// No PID file → ensure_no_running_daemon succeeds.
#[test]
fn test_ensure_no_running_daemon_no_file() {
    let tmp = TempDir::new().unwrap();
    let pid_file = pid_file_path(tmp.path());
    assert!(ensure_no_running_daemon(&pid_file).is_ok());
}

/// Stale PID file → ensure_no_running_daemon succeeds (file is cleaned).
#[test]
fn test_ensure_no_running_daemon_stale() {
    let tmp = TempDir::new().unwrap();
    let pid_file = pid_file_path(tmp.path());
    write_pid_file(&pid_file, 99999999).unwrap();
    assert!(pid_file.exists(), "PID file should exist before check");

    let result = ensure_no_running_daemon(&pid_file);
    assert!(result.is_ok(), "stale PID should not block: {result:?}");
    assert!(!pid_file.exists(), "stale PID file should be removed");
}

/// Alive PID → ensure_no_running_daemon returns error.
#[test]
fn test_ensure_no_running_daemon_alive() {
    let tmp = TempDir::new().unwrap();
    let pid_file = pid_file_path(tmp.path());
    let my_pid = std::process::id();
    write_pid_file(&pid_file, my_pid).unwrap();

    let result = ensure_no_running_daemon(&pid_file);
    assert!(result.is_err(), "should error when daemon is alive");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("daemon already running"),
        "error should mention daemon already running, got: {err_msg}"
    );
    assert!(
        err_msg.contains(&my_pid.to_string()),
        "error should include the alive PID, got: {err_msg}"
    );
    // PID file should NOT be removed for an alive process.
    assert!(
        pid_file.exists(),
        "PID file should be preserved for alive process"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// prepare_run path expansion tests
// ═══════════════════════════════════════════════════════════════════════════
//
// These tests verify the behavioral dimensions required by the plan:
// - Normal paths: relative preserved, ~ expanded to absolute home
// - Env var expansion: $VAR/${VAR} expanded via subprocess
//   (env mutation prohibited by CONTRIBUTING.md §7)
// - Edge cases: empty string → root_dir(), bare ~ → home
// - Error propagation: root_dir() failure

/// Relative path without shorthand is preserved as-is by expand_path
/// (expand_home / expand_env are no-ops for paths without ~ or $).
/// expand_path performs no semantic change on /-separated paths that lack
/// ~ or $ prefixes.
#[test]
fn test_prepare_run_relative_path_preserved() {
    let (config_dir, pid_file) = prepare_run("some/relative/path").unwrap();
    let expected = PathBuf::from("some/relative/path");
    assert_eq!(
        config_dir, expected,
        "relative path should be unchanged (expand_path is a no-op for paths without ~ or $)"
    );
    assert!(
        pid_file.starts_with(&expected),
        "pid_file should be under the config_dir"
    );
}

/// Absolute path (no shorthand) passes through expand_path idempotently.
#[test]
fn test_prepare_run_absolute_path_idempotent() {
    let (config_dir, _pid_file) = prepare_run("/tmp/absolute/test").unwrap();
    assert_eq!(
        config_dir,
        PathBuf::from("/tmp/absolute/test"),
        "absolute path without shorthand should be unchanged"
    );
    assert!(config_dir.is_absolute());
}

/// `~` prefix expands to the user's actual home directory.
/// The expanded path must be absolute and must NOT start with `~`.
#[test]
fn test_prepare_run_tilde_expands_to_absolute_home() {
    let (config_dir, _pid_file) = prepare_run("~/conf").unwrap();
    assert!(
        config_dir.is_absolute(),
        "expanded ~ path should be absolute, got: {}",
        config_dir.display()
    );
    assert!(
        !config_dir.to_string_lossy().starts_with('~'),
        "expanded path should not start with ~, got: {}",
        config_dir.display()
    );
    // Should end with /conf (the suffix after ~/).
    assert!(
        config_dir.ends_with("conf"),
        "expanded path should retain the suffix, got: {}",
        config_dir.display()
    );
}

/// Empty string falls back to root_dir() default (same as no --config-dir).
#[test]
fn test_prepare_run_empty_string_uses_root_dir_default() {
    let (config_dir_empty, _) = prepare_run("").unwrap();
    let (config_dir_default, _) = prepare_run("").unwrap();
    // Both calls should produce the same root_dir() result.
    assert_eq!(
        config_dir_empty, config_dir_default,
        "empty string should consistently use root_dir() default"
    );
    assert!(
        config_dir_empty.is_absolute(),
        "root_dir() should return an absolute path"
    );
}

/// Bare `~` expands to the home directory itself (no suffix appended).
#[test]
fn test_prepare_run_bare_tilde_is_home() {
    let (config_dir, _pid_file) = prepare_run("~").unwrap();
    assert!(
        config_dir.is_absolute(),
        "bare ~ should expand to an absolute home path, got: {}",
        config_dir.display()
    );
    assert!(
        !config_dir.to_string_lossy().ends_with('~'),
        "bare ~ should not remain literal, got: {}",
        config_dir.display()
    );
    // The result should be exactly the home directory (no extra segments).
    let home = dirs::home_dir().expect("HOME should be available in test env");
    assert_eq!(
        config_dir, home,
        "bare ~ should resolve to home dir exactly"
    );
}

// ── Env var expansion tests (subprocess-based) ────────────────────────────
// Env mutation is prohibited by CONTRIBUTING.md §7,
// so env var expansion is tested by spawning a subprocess with the
// target env set and parsing the output.

/// Run the helper binary with optional env vars set and return its stdout.
/// On failure, outputs both stdout and stderr for debugging.
fn run_helper(helper: &std::path::Path, config_dir: &str, envs: &[(&str, &str)]) -> String {
    let mut cmd = std::process::Command::new(helper);
    cmd.arg(config_dir);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("failed to execute helper binary");
    assert!(
        output.status.success(),
        "helper binary failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("helper output is not UTF-8")
}

/// Helper binary source: compiled into a temporary crate at test time.
/// Prints the config_dir path returned by prepare_run.
/// Panics immediately if config_dir argument is missing.
const HELPER_SRC: &str = r#"fn main() {
    let config_dir = std::env::args().nth(1).expect("missing config_dir argument");
    let (resolved, _) = closeclaw_cli::admin::prepare_run(&config_dir).unwrap();
    println!("{}", resolved.display());
}"#;

// ── Helper infrastructure for env var tests ────────────────────────────────
// Compiled once via OnceLock so all env var test cases share a single build
// (CI red line: single case >5s must fix).
static HELPER: OnceLock<(tempfile::TempDir, std::path::PathBuf)> = OnceLock::new();

fn helper_path() -> &'static std::path::Path {
    &HELPER.get_or_init(create_helper_project).1
}

/// Write a minimal Cargo project that depends on closeclaw-cli and compiles
/// the helper binary. Returns (temp_dir, binary_path).
fn create_helper_project() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let proj = tmp.path().join("helper_proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();

    // Find workspace root for path dependencies.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let cli_path = workspace_root.join("crates").join("cli");
    // Write Cargo.toml with path dep to closeclaw-cli.
    let cargo_toml = format!(
        r#"[package]
name = "prepare_run_helper"
version = "0.0.0"
edition = "2021"

[dependencies]
closeclaw-cli = {{ path = "{}" }}"#,
        cli_path.display()
    );
    std::fs::write(proj.join("Cargo.toml"), &cargo_toml).unwrap();
    std::fs::write(proj.join("src").join("main.rs"), HELPER_SRC).unwrap();

    // Build.
    let status = std::process::Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(proj.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(tmp.path().join("target"))
        .current_dir(&workspace_root)
        .status()
        .expect("failed to compile helper binary");
    assert!(status.success(), "helper binary compilation failed");

    let helper = tmp
        .path()
        .join("target")
        .join("debug")
        .join("prepare_run_helper");
    assert!(
        helper.exists(),
        "helper binary not found at {}",
        helper.display()
    );

    (tmp, helper)
}

/// Env var expansion: table-driven test covering all cases.
/// Helper binary is compiled once via OnceLock and shared across all cases.
/// Cases: (input, env vars, expected output)
#[test]
fn test_prepare_run_env_var_expansion() {
    let helper = helper_path();
    let cases: &[(&str, &[(&str, &str)], &str)] = &[
        // $TESTVAR is expanded to its value when set
        (
            "$TESTVAR/sub",
            &[("TESTVAR", "/opt/testval")],
            "/opt/testval/sub",
        ),
        // ${TESTVAR} brace syntax is also expanded
        (
            "${TESTVAR}/sub",
            &[("TESTVAR", "/opt/braced")],
            "/opt/braced/sub",
        ),
        // Undefined env var is preserved as literal text
        ("$UNDEFINED_XYZ_12345/sub", &[], "$UNDEFINED_XYZ_12345/sub"),
    ];
    for (input, envs, expected) in cases {
        let result = run_helper(helper, input, envs);
        assert_eq!(
            result.trim(),
            *expected,
            "env var expansion failed for input={input}"
        );
    }
}

/// root_dir() failure (HOME unset) propagates as an error.
/// When HOME is unset, root_dir() returns an error, and prepare_run(""")
/// must propagate it rather than panicking.
#[test]
fn test_prepare_run_root_dir_failure_propagates() {
    let (tmp, helper) = create_helper_project();
    let output = std::process::Command::new(&helper)
        .arg("")
        .env_remove("HOME")
        .output()
        .expect("failed to execute helper binary");
    assert!(
        !output.status.success(),
        "should fail when HOME is unset and config_dir is empty"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("HOME") || stderr.contains("environment variable"),
        "error should reference HOME or environment variable, got: {stderr}"
    );
    drop(tmp);
}
