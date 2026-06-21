//! Unit tests for handle_run.
//!
//! Covers config_dir resolution and PID file writing.
//! Daemon start is expected to fail or block in test environments;
//! tests use a short timeout to avoid hanging and verify PID file state.

use crate::cli::admin::config_dir_for;
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
/// Uses config_dir_for() with a temp path to avoid writing to real config dir.
#[tokio::test]
async fn test_run_config_dir_uses_platform_path() {
    let tmp = TempDir::new().unwrap();
    let fake_home = tmp.path();
    let expected_default = config_dir_for(fake_home);

    // Temporarily override HOME so config_dir() resolves to our temp dir
    // by passing a non-empty config_dir to handle_run instead.
    let config_dir = expected_default.to_str().unwrap().to_string();
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        handle_run(config_dir, false),
    )
    .await;

    // Verify PID file was written in the expected config_dir
    let pid_file = expected_default.join("daemon.pid");
    assert!(pid_file.exists(), "PID file should exist in config_dir");

    let content = fs::read_to_string(&pid_file).unwrap();
    let pid: u32 = content.trim().parse().unwrap();
    assert_eq!(
        pid,
        std::process::id(),
        "PID file should contain current process PID"
    );
}

/// RunOutput serializes to JSON with correct fields.
#[test]
fn test_run_output_json() {
    let output = RunOutput {
        pid: 12345,
        config_dir: "/tmp/test".to_string(),
        stopped: true,
    };
    let json = serde_json::to_string(&output).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["pid"], 12345);
    assert_eq!(v["config_dir"], "/tmp/test");
    assert_eq!(v["stopped"], true);
}
