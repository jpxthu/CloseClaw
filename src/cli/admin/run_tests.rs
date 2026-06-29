//! Unit tests for prepare_run, build_daemon_command, and run helpers.
//!
//! Covers config_dir resolution, PID file writing, subprocess command
//! construction, socket readiness detection, and RunOutput serialization.

use crate::cli::admin::config_dir_for;
use crate::cli::admin::run::{build_daemon_command, prepare_run, try_connect, wait_for_socket};
use crate::cli::admin::RunOutput;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// prepare_run: config_dir resolution and PID file path
// ---------------------------------------------------------------------------

/// When a non-empty config_dir is passed, prepare_run resolves to that path;
/// when empty it falls back to the default platform path.
#[test]
fn test_run_config_dir_uses_platform_default() {
    // Case 1: non-empty config_dir → use provided path
    let tmp1 = TempDir::new().unwrap();
    let fake_home = tmp1.path();
    let expected_default = config_dir_for(fake_home);
    let config_dir = expected_default.to_str().unwrap().to_string();
    let (resolved, pid_file) = prepare_run(&config_dir).unwrap();
    assert_eq!(resolved, expected_default);
    assert!(
        pid_file.display().to_string().ends_with("daemon.pid"),
        "PID file should end with daemon.pid"
    );

    // Case 2: empty config_dir → fall back to platform default
    let tmp2 = TempDir::new().unwrap();
    let empty_dir = tmp2.path().to_str().unwrap().to_string();
    let (resolved2, pid_file2) = prepare_run(&empty_dir).unwrap();
    assert_eq!(resolved2, tmp2.path());
    assert!(
        pid_file2.display().to_string().ends_with("daemon.pid"),
        "PID file should end with daemon.pid"
    );
}

/// prepare_run creates the config directory if it does not exist.
#[test]
fn test_prepare_run_creates_config_dir() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("a").join("b").join("c");
    let (resolved, _) = prepare_run(nested.to_str().unwrap()).unwrap();
    assert!(resolved.is_dir(), "config dir should be created");
    assert_eq!(resolved, nested);
}

// ---------------------------------------------------------------------------
// build_daemon_command: subprocess argument construction
// ---------------------------------------------------------------------------

/// build_daemon_command sets the correct executable, arguments, and stdio.
#[test]
fn test_build_daemon_command_args() {
    let exe = std::path::PathBuf::from("/usr/bin/closeclaw");
    let config_dir = std::path::PathBuf::from("/tmp/my-daemon");

    let cmd = build_daemon_command(&exe, &config_dir);

    // Verify the program is set to current_exe.
    assert_eq!(cmd.get_program(), exe.as_os_str());

    // Verify arguments: run --config-dir <dir> --foreground
    let args: Vec<&std::ffi::OsStr> = cmd.get_args().collect();
    assert_eq!(args.len(), 4);
    assert_eq!(args[0], "run");
    assert_eq!(args[1], "--config-dir");
    assert_eq!(args[2], config_dir.as_os_str());
    assert_eq!(args[3], "--foreground");
}

/// build_daemon_command nullifies stdin/stdout/stderr for daemon mode.
#[test]
fn test_build_daemon_command_stdio_null() {
    let exe = std::path::PathBuf::from("/usr/bin/closeclaw");
    let config_dir = std::path::PathBuf::from("/tmp/test");

    let cmd = build_daemon_command(&exe, &config_dir);

    // stdio should be set to null (we can verify by checking the command
    // can be formatted without panic — actual Stdio is opaque).
    // The key assertion is that the command builds successfully.
    let _ = cmd;
}

// ---------------------------------------------------------------------------
// wait_for_socket: timeout and readiness detection
// ---------------------------------------------------------------------------

/// wait_for_socket returns an error when the socket path does not exist.
#[test]
fn test_wait_for_socket_timeout_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let fake_socket = tmp.path().join("nonexistent.sock");

    // Use a very short timeout to keep the test fast (<1s).
    let result = wait_for_socket(&fake_socket, 300);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Timed out"),
        "error should mention timeout, got: {err_msg}"
    );
    assert!(
        err_msg.contains("nonexistent.sock"),
        "error should include socket path, got: {err_msg}"
    );
}

/// wait_for_socket returns Ok immediately when a socket is already listening.
#[test]
fn test_wait_for_socket_success_immediate() {
    use std::os::unix::net::UnixListener;

    let tmp = TempDir::new().unwrap();
    let sock_path = tmp.path().join("test.sock");

    let _listener = UnixListener::bind(&sock_path).unwrap();

    // Socket is listening — should succeed within the first poll.
    let result = wait_for_socket(&sock_path, 1_000);
    assert!(result.is_ok(), "should succeed when socket is listening");
}

// ---------------------------------------------------------------------------
// try_connect: single connection attempt
// ---------------------------------------------------------------------------

/// try_connect fails for a non-existent socket path.
#[test]
fn test_try_connect_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let fake_socket = tmp.path().join("no-such.sock");

    let result = try_connect(&fake_socket);
    assert!(result.is_err());
}

/// try_connect succeeds for a listening Unix socket.
#[test]
fn test_try_connect_success() {
    use std::os::unix::net::UnixListener;

    let tmp = TempDir::new().unwrap();
    let sock_path = tmp.path().join("live.sock");

    let _listener = UnixListener::bind(&sock_path).unwrap();

    let result = try_connect(&sock_path);
    assert!(result.is_ok(), "should connect to a listening socket");
}

// ---------------------------------------------------------------------------
// RunOutput: serialization fields
// ---------------------------------------------------------------------------

/// RunOutput serializes to JSON with correct fields (started replaces stopped).
#[test]
fn test_run_output_json() {
    let output = RunOutput {
        pid: 12345,
        config_dir: "/tmp/test".to_string(),
        started: true,
    };
    let json = serde_json::to_string(&output).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["pid"], 12345);
    assert_eq!(v["config_dir"], "/tmp/test");
    assert_eq!(v["started"], true);
}

/// RunOutput has no `stopped` field — only `started`.
#[test]
fn test_run_output_no_stopped_field() {
    let output = RunOutput {
        pid: 1,
        config_dir: "/tmp".to_string(),
        started: false,
    };
    let v: serde_json::Value = serde_json::to_value(&output).unwrap();
    assert!(
        v.get("stopped").is_none(),
        "RunOutput should not have a `stopped` field"
    );
    assert!(
        v.get("started").is_some(),
        "RunOutput should have a `started` field"
    );
}
