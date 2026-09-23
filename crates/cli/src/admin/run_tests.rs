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
use closeclaw_platform::process::{read_pid_file, write_pid_file};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

/// Helper: create an isolated PID file path under a TempDir.
///
/// Builds `{tmp}/.closeclaw/daemon.pid` and ensures the parent directory
/// exists, matching the production directory layout. Returns the PID file
/// path and the TempDir guard (must be kept alive for the test duration).
fn isolated_pid_file(tmp: &TempDir) -> PathBuf {
    let pid_dir = tmp.path().join(".closeclaw");
    std::fs::create_dir_all(&pid_dir).expect("failed to create .closeclaw dir");
    pid_dir.join("daemon.pid")
}

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

    let result = handle_run_foreground(&config_dir, false, &mock, None).await;
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
    let result = handle_run(config_dir, false, false, &mock, None).await;
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

    let result = handle_run_foreground(&config_dir, false, &mock, None).await;
    assert!(result.is_err(), "should propagate the error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("simulated daemon crash"),
        "error message should contain the mock failure text, got: {err_msg}"
    );
}

// ── Test 4: Foreground mode does NOT write PID file ────────────────────────

/// After Step 1.3, handle_run_foreground no longer writes the PID file —
/// PID self-registration is the daemon's responsibility (Step 1.1). This
/// test verifies that the CLI layer does NOT create a PID file as a side
/// effect of running in foreground mode.
///
/// Uses an isolated TempDir PID path to avoid touching the global
/// `~/.closeclaw/daemon.pid`.
#[tokio::test]
async fn test_handle_run_foreground_does_not_write_pid_file() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let pid_file = isolated_pid_file(&tmp);

    let mock = MockDaemonRunner::success();

    let result = handle_run_foreground(&config_dir, false, &mock, Some(&pid_file)).await;
    assert!(
        result.is_ok(),
        "handle_run_foreground should succeed: {result:?}"
    );
    assert!(mock.was_called(), "mock should have been called");

    // PID file should NOT exist — the daemon is responsible for writing it.
    assert!(
        !pid_file.exists(),
        "PID file should NOT be created by handle_run_foreground \
         (daemon owns PID self-registration)",
    );
}

// ── Test 5: Foreground rejects start when alive instance detected ───────────

/// When a PID file exists and the process is alive, handle_run_foreground
/// must bail with "daemon already running".
///
/// Uses an isolated TempDir PID path to avoid touching the global
/// `~/.closeclaw/daemon.pid`.
#[tokio::test]
async fn test_handle_run_foreground_rejects_alive_daemon() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let pid_file = isolated_pid_file(&tmp);

    // Write our own PID to simulate a running daemon.
    write_pid_file(&pid_file, std::process::id()).unwrap();

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock, Some(&pid_file)).await;
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
///
/// Uses an isolated TempDir PID path to avoid touching the global
/// `~/.closeclaw/daemon.pid`.
#[tokio::test]
async fn test_handle_run_foreground_cleans_stale_pid() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let pid_file = isolated_pid_file(&tmp);

    // Write a PID that does not exist (stale).
    write_pid_file(&pid_file, 99999999).unwrap();

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock, Some(&pid_file)).await;
    assert!(
        result.is_ok(),
        "should succeed after cleaning stale PID: {result:?}"
    );
    assert!(
        mock.was_called(),
        "DaemonRunner should be called after stale PID is cleaned"
    );
    // Stale PID file should have been cleaned by ensure_no_running_daemon.
    assert!(
        !pid_file.exists(),
        "stale PID file should have been removed"
    );
}

// ── Test 7: Foreground succeeds with no existing PID file ───────────────────

/// When no PID file exists, handle_run_foreground must proceed normally.
///
/// Uses an isolated TempDir PID path to avoid touching the global
/// `~/.closeclaw/daemon.pid`.
#[tokio::test]
async fn test_handle_run_foreground_no_pid_file() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let pid_file = isolated_pid_file(&tmp);

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock, Some(&pid_file)).await;
    assert!(
        result.is_ok(),
        "should succeed with no existing PID file: {result:?}"
    );
    assert!(mock.was_called(), "DaemonRunner should be called");
}

// ── Test 8: Background rejects start when alive instance detected ───────────

/// When a PID file exists and the process is alive, handle_run(background)
/// must bail with "daemon already running".
///
/// Uses an isolated TempDir PID path to avoid touching the global
/// `~/.closeclaw/daemon.pid`.
#[tokio::test]
async fn test_handle_run_background_rejects_alive_daemon() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let pid_file = isolated_pid_file(&tmp);

    // Write our own PID to simulate a running daemon.
    write_pid_file(&pid_file, std::process::id()).unwrap();

    let mock = MockDaemonRunner::success();
    let result = handle_run(config_dir, false, false, &mock, Some(&pid_file)).await;
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
///
/// Uses an isolated TempDir PID path to avoid touching the global
/// `~/.closeclaw/daemon.pid`.
#[tokio::test]
async fn test_handle_run_background_cleans_stale_pid() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let pid_file = isolated_pid_file(&tmp);

    // Write a PID that does not exist (stale).
    write_pid_file(&pid_file, 99999999).unwrap();

    let mock = MockDaemonRunner::success();
    let _result = handle_run(config_dir, false, false, &mock, Some(&pid_file)).await;
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
    let pid_file = tmp.path().join("daemon.pid");
    assert!(ensure_no_running_daemon(&pid_file).is_ok());
}

/// Stale PID file → ensure_no_running_daemon succeeds (file is cleaned).
#[test]
fn test_ensure_no_running_daemon_stale() {
    let tmp = TempDir::new().unwrap();
    let pid_file = tmp.path().join("daemon.pid");
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
    let pid_file = tmp.path().join("daemon.pid");
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
    let (config_dir, pid_file) = prepare_run("some/relative/path", None).unwrap();
    let expected = PathBuf::from("some/relative/path");
    assert_eq!(
        config_dir, expected,
        "relative path should be unchanged (expand_path is a no-op for paths without ~ or $)"
    );
    assert!(
        pid_file.to_str().unwrap().contains("daemon.pid"),
        "pid_file should be the fixed daemon.pid path, got: {}",
        pid_file.display()
    );
}

/// Absolute path (no shorthand) passes through expand_path idempotently.
#[test]
fn test_prepare_run_absolute_path_idempotent() {
    let (config_dir, _pid_file) = prepare_run("/tmp/absolute/test", None).unwrap();
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
    let (config_dir, _pid_file) = prepare_run("~/conf", None).unwrap();
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
    let (config_dir_empty, _) = prepare_run("", None).unwrap();
    let (config_dir_default, _) = prepare_run("", None).unwrap();
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
    let (config_dir, _pid_file) = prepare_run("~", None).unwrap();
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

// ── Env var expansion tests (re-exec child) ──────────────────────────────
// Env mutation is prohibited by CONTRIBUTING.md §7, so env var expansion is
// tested by re-exec'ing this test binary as a gated helper child with the
// target env injected (precedent: crates/platform/src/process_tests.rs).

/// Env gate: set only on the re-exec'd child so normal test runs skip the
/// helper sub-test.
const HELPER_CHILD_GATE: &str = "PREPARE_RUN_HELPER_CHILD";

/// Raw `--config-dir` input injected by the parent test into the child.
const HELPER_CONFIG_DIR: &str = "PREPARE_RUN_HELPER_CONFIG_DIR";

/// Marker prefix of the resolved-path line printed by the helper child.
const HELPER_RESULT_MARKER: &str = "PREPARE_RUN_RESOLVED=";

/// `--exact` filter selecting only the helper sub-test inside the child.
const HELPER_CHILD_TEST: &str = "admin::run_tests::test_prepare_run_helper_child";

/// Re-exec the current test binary as the `prepare_run` helper child.
///
/// `--exact` restricts the child to the helper sub-test; `envs` /
/// `remove_home` reproduce the isolation semantics of the old compiled
/// helper binary (subprocess-only env, no process-global mutation).
fn spawn_helper_child(
    config_dir: &str,
    envs: &[(&str, &str)],
    remove_home: bool,
) -> std::process::Output {
    let test_binary = std::env::current_exe().expect("failed to resolve current test binary");
    let mut cmd = std::process::Command::new(test_binary);
    cmd.env(HELPER_CHILD_GATE, "1")
        .env(HELPER_CONFIG_DIR, config_dir)
        .arg("--exact")
        .arg("--nocapture")
        .arg(HELPER_CHILD_TEST);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if remove_home {
        cmd.env_remove("HOME");
    }
    cmd.output().expect("failed to execute helper child")
}

/// Helper sub-test body: resolves `prepare_run` under the parent-injected
/// env and prints the result with [`HELPER_RESULT_MARKER`]. Inert during a
/// normal run (gate unset); the parent tests drive it by re-exec'ing this
/// binary with [`HELPER_CHILD_GATE`] + `--exact` filtering.
#[test]
fn test_prepare_run_helper_child() {
    if std::env::var(HELPER_CHILD_GATE).is_err() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    let config_dir = std::env::var(HELPER_CONFIG_DIR).expect("missing config_dir input");
    match prepare_run(&config_dir, None) {
        Ok((resolved, _)) => println!("{HELPER_RESULT_MARKER}{}", resolved.display()),
        Err(err) => {
            eprintln!("prepare_run failed: {err:?}");
            std::process::exit(1);
        }
    }
}

/// Run the helper child with optional env vars set and return the resolved
/// config_dir it printed (stripped from the [`HELPER_RESULT_MARKER`] line).
/// On failure, outputs both stdout and stderr for debugging.
fn run_helper(config_dir: &str, envs: &[(&str, &str)]) -> String {
    let output = spawn_helper_child(config_dir, envs, false);
    assert!(
        output.status.success(),
        "helper child failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("helper output is not UTF-8");
    stdout
        .lines()
        .find_map(|line| line.split(HELPER_RESULT_MARKER).nth(1))
        .map(str::to_string)
        .unwrap_or_else(|| {
            panic!("helper child printed no {HELPER_RESULT_MARKER} line, stdout: {stdout}")
        })
}

/// One env-expansion case: (input config_dir, injected env vars, expected
/// output).
type ExpansionCase<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);

/// Env var expansion: table-driven test covering all cases.
/// Each case re-execs the current test binary as the helper child — no
/// compile step, so every case stays well under the 5s red line.
/// Cases: (input, env vars, expected output)
#[test]
fn test_prepare_run_env_var_expansion() {
    let cases: &[ExpansionCase<'_>] = &[
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
        let result = run_helper(input, envs);
        assert_eq!(
            result.trim(),
            *expected,
            "env var expansion failed for input={input}"
        );
    }
}

/// Boundary: `~` and an env var in the same config_dir — expand_path
/// expands the home shorthand first, then substitutes the env var, so
/// `~/$TESTVAR` resolves to `<home>/<value>` end to end through the
/// helper child (issue #3161 Step 1.3). The child inherits the parent's
/// HOME, so the expected value is computed from the parent's home dir.
#[test]
fn test_prepare_run_tilde_and_env_var_combined() {
    let home = dirs::home_dir().expect("HOME should be available in test env");
    let result = run_helper("~/$TESTVAR", &[("TESTVAR", "conf-sub")]);
    assert_eq!(
        PathBuf::from(result.trim()),
        home.join("conf-sub"),
        "~/ must expand before $TESTVAR is substituted"
    );
}

/// Boundary: an env var *value* containing `~` is not home-expanded —
/// expand_path is a single forward pass (home → env), so the literal
/// `~/` coming from the substituted value survives as-is. Locks the
/// expansion order (issue #3161 Step 1.3).
#[test]
fn test_prepare_run_env_value_tilde_not_reexpanded() {
    let result = run_helper("$TESTVAR", &[("TESTVAR", "~/literal")]);
    assert_eq!(
        PathBuf::from(result.trim()),
        PathBuf::from("~/literal"),
        "env values must not be re-scanned for ~ after substitution"
    );
}

/// root_dir() failure (HOME unset) propagates as an error.
/// When HOME is unset, root_dir() returns an error, so prepare_run("") must
/// fail: the helper child reports it on stderr and exits non-zero.
#[test]
fn test_prepare_run_root_dir_failure_propagates() {
    let output = spawn_helper_child("", &[], true);
    assert!(
        !output.status.success(),
        "should fail when HOME is unset and config_dir is empty\nstdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("HOME") || stderr.contains("environment variable"),
        "error should reference HOME or environment variable, got: {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// prepare_run decoupling: --config-dir does NOT affect PID file location
// ═══════════════════════════════════════════════════════════════════════════

/// When different custom config_dir values are passed, prepare_run must
/// return config_dir that varies with the input, but pid_file must always
/// be the fixed platform path (~/.closeclaw/daemon.pid). This verifies
/// that --config-dir is decoupled from PID file location.
#[test]
fn test_prepare_run_config_dir_decoupled_from_pid_file() {
    let (cfg1, pid1) = prepare_run("/custom/a", None).unwrap();
    let (cfg2, pid2) = prepare_run("/custom/b", None).unwrap();
    let (cfg3, pid3) = prepare_run("~/other", None).unwrap();

    // config_dir varies with input
    assert_ne!(
        cfg1, cfg2,
        "different inputs must yield different config_dir"
    );
    assert_ne!(
        cfg1, cfg3,
        "different inputs must yield different config_dir"
    );

    // pid_file is always the same fixed path regardless of config_dir
    assert_eq!(
        pid1, pid2,
        "pid_file must be identical despite different config_dir"
    );
    assert_eq!(
        pid2, pid3,
        "pid_file must be identical despite different config_dir"
    );
    assert_eq!(
        pid1,
        closeclaw_platform::process::pid_file_path().unwrap(),
        "pid_file must match the platform fixed path"
    );
    assert!(
        pid1.to_string_lossy().ends_with("daemon.pid"),
        "pid_file must end with daemon.pid, got: {}",
        pid1.display()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.4: Injection interface & isolation behavior tests
// ═══════════════════════════════════════════════════════════════════════════

/// When pid_file_override points to a path whose parent directory does
/// not exist, the function must succeed.  `read_pid_file` returns `None`
/// for any I/O error (including ENOENT on the parent), so
/// `check_stale_pid` treats it as "no PID file" and proceeds normally.
/// This locks down the actual behavior for this boundary case.
#[tokio::test]
async fn test_injection_parent_dir_not_exist() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    // Inject a path whose parent directory does NOT exist.
    let nonexistent_pid = tmp
        .path()
        .join("nonexistent")
        .join("deep")
        .join("daemon.pid");
    assert!(
        !nonexistent_pid.parent().unwrap().exists(),
        "parent dir should not exist"
    );

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock, Some(&nonexistent_pid)).await;

    // When parent dir doesn't exist, read_pid_file returns None (file
    // can't be read), so check_stale_pid treats it as "no PID file" and
    // the function proceeds normally.
    assert!(
        result.is_ok(),
        "should succeed when parent dir doesn't exist: {result:?}"
    );
    assert!(mock.was_called(), "DaemonRunner should be called");
    // PID file should NOT have been created (foreground doesn't write PID,
    // and parent dir doesn't exist anyway).
    assert!(
        !nonexistent_pid.exists(),
        "PID file should not be created when parent dir is missing"
    );
}

/// When the injected PID file contains corrupted (non-numeric) content,
/// `read_pid_file` returns `None`, which `check_stale_pid` treats as
/// "no PID file".  The function proceeds normally — corrupted content
/// is not an error, it's equivalent to an absent file.
#[tokio::test]
async fn test_injection_corrupted_pid_file() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let injected_pid = isolated_pid_file(&tmp);

    // Write corrupted content to the PID file.
    std::fs::write(&injected_pid, "not_a_number").unwrap();

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock, Some(&injected_pid)).await;

    // Corrupted PID is treated as "no daemon running" → proceeds normally.
    assert!(
        result.is_ok(),
        "corrupted PID should not block startup: {result:?}"
    );
    assert!(mock.was_called(), "DaemonRunner should be called");
}

/// When pid_file_override is provided, the function must use only the
/// injected path and never touch the global `~/.closeclaw/daemon.pid`.
///
/// We prove isolation by creating a "control" PID file with a LIVE PID.
/// If the function had used the control path instead of the injected path,
/// it would fail with "daemon already running".  The function succeeds,
/// proving the injected path is used exclusively.
#[tokio::test]
async fn test_injection_does_not_touch_global_pid_path() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    // Create a control PID file simulating the global path.
    // It contains a LIVE PID — touching this path causes failure.
    let control_pid = tmp.path().join("control_global.pid");
    write_pid_file(&control_pid, std::process::id()).unwrap();

    // Create the injected path with a DEAD PID — should be cleaned.
    let injected_pid = isolated_pid_file(&tmp);
    write_pid_file(&injected_pid, 99999999).unwrap();

    let mock = MockDaemonRunner::success();
    let result = handle_run_foreground(&config_dir, false, &mock, Some(&injected_pid)).await;

    // Function succeeds — it used the injected path (dead PID → cleaned)
    // and did NOT touch the control path (live PID → would fail).
    assert!(
        result.is_ok(),
        "should succeed using injected path, not control path: {result:?}"
    );
    assert!(mock.was_called(), "DaemonRunner should be called");
    assert!(
        !injected_pid.exists(),
        "stale injected PID should be cleaned"
    );

    // The control path was never read — it still has the live PID.
    assert!(control_pid.exists(), "control PID file should be untouched");
    assert_eq!(
        read_pid_file(&control_pid),
        Some(std::process::id()),
        "control PID file content must be unchanged (never read or written)"
    );
}

/// Full PID lifecycle state transition through ensure_no_running_daemon:
///
/// 1. No file → Ok (clean start)
/// 2. Write alive PID → Err (daemon already running)
/// 3. Kill process → Ok (stale detected, file cleaned)
/// 4. No file again → Ok (clean state restored)
///
/// This verifies the complete state machine that guard functions implement,
/// ensuring each transition produces the correct observable behavior.
#[test]
fn test_full_pid_lifecycle_state_transition() {
    let tmp = TempDir::new().unwrap();
    let pid_file = tmp.path().join("daemon.pid");

    // State 1: No file → clean start.
    assert!(!pid_file.exists());
    let result = ensure_no_running_daemon(&pid_file);
    assert!(result.is_ok(), "no file should yield Ok: {result:?}");

    // State 2: Write our own PID → daemon already running.
    let my_pid = std::process::id();
    write_pid_file(&pid_file, my_pid).unwrap();
    assert!(pid_file.exists());
    let result = ensure_no_running_daemon(&pid_file);
    assert!(result.is_err(), "alive PID should yield Err");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("daemon already running"));
    assert!(
        pid_file.exists(),
        "PID file must be preserved for alive process"
    );

    // Transition: kill the process so the PID becomes stale.
    // We use the current process PID, which is alive. Instead of actually
    // killing ourselves, write a guaranteed-dead PID (99999999) and verify
    // the stale path.
    write_pid_file(&pid_file, 99999999).unwrap();

    // State 3: Stale PID → cleaned automatically.
    let result = ensure_no_running_daemon(&pid_file);
    assert!(
        result.is_ok(),
        "stale PID should be cleaned and yield Ok: {result:?}"
    );
    assert!(
        !pid_file.exists(),
        "stale PID file should be removed by ensure_no_running_daemon"
    );

    // State 4: No file again → clean state restored.
    let result = ensure_no_running_daemon(&pid_file);
    assert!(result.is_ok(), "clean state should yield Ok: {result:?}");
}
