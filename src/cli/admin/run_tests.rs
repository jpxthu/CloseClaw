//! Unit tests for handle_run.
//!
//! Covers config_dir resolution and PID file writing.
//! Daemon start is expected to fail or block in test environments;
//! tests use a short timeout to avoid hanging and verify PID file state.

use crate::cli::admin::handle_run;
use crate::cli::admin::RunOutput;
use std::fs;
use tempfile::TempDir;

/// When config_dir is specified, handle_run uses that path and writes PID.
#[tokio::test]
async fn test_run_specified_config_dir() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    // handle_run writes PID file then starts daemon; use timeout to avoid
    // hanging on the daemon event loop.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        handle_run(config_dir, false),
    )
    .await;

    // Verify PID file was written in the specified config_dir
    let pid_file = tmp.path().join("daemon.pid");
    assert!(
        pid_file.exists(),
        "PID file should exist in specified config_dir"
    );

    let content = fs::read_to_string(&pid_file).unwrap();
    let pid: u32 = content.trim().parse().unwrap();
    assert_eq!(
        pid,
        std::process::id(),
        "PID file should contain current process PID"
    );
}

/// When config_dir is empty, handle_run resolves to the default platform path.
#[tokio::test]
#[serial_test::serial]
async fn test_run_empty_config_dir_uses_default() {
    let expected_default = crate::platform::config::config_dir().unwrap();

    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        handle_run(String::new(), false),
    )
    .await;

    // Verify PID file was written in the default config_dir
    let pid_file = expected_default.join("daemon.pid");
    assert!(
        pid_file.exists(),
        "PID file should exist in default config_dir"
    );

    let content = fs::read_to_string(&pid_file).unwrap();
    let pid: u32 = content.trim().parse().unwrap();
    assert_eq!(
        pid,
        std::process::id(),
        "PID file should contain current process PID"
    );
}

/// PID file is written correctly before daemon start.
#[tokio::test]
async fn test_run_pid_file_written() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();

    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        handle_run(config_dir, false),
    )
    .await;

    let pid_file = tmp.path().join("daemon.pid");
    assert!(pid_file.exists(), "PID file should be created");

    let content = fs::read_to_string(&pid_file).unwrap();
    let pid: u32 = content.trim().parse().unwrap();
    assert_eq!(pid, std::process::id());
}

/// RunOutput serializes to JSON with correct fields.
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
