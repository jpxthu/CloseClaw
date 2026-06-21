//! Unit tests for prepare_run.
//!
//! Covers config_dir resolution and PID file writing without starting
//! a real daemon.

use crate::cli::admin::config_dir_for;
use crate::cli::admin::run::prepare_run;
use crate::cli::admin::RunOutput;
use std::fs;
use tempfile::TempDir;

/// When a non-empty config_dir is passed, prepare_run resolves to that path.
#[test]
fn test_run_config_dir_uses_platform_path() {
    let tmp = TempDir::new().unwrap();
    let fake_home = tmp.path();
    let expected_default = config_dir_for(fake_home);

    let config_dir = expected_default.to_str().unwrap().to_string();
    let (resolved, pid) = prepare_run(&config_dir).unwrap();

    assert_eq!(resolved, expected_default);
    assert_eq!(pid, std::process::id());

    // Verify PID file was written in the expected config_dir
    let pid_file = expected_default.join("daemon.pid");
    assert!(pid_file.exists(), "PID file should exist in config_dir");

    let content = fs::read_to_string(&pid_file).unwrap();
    let written_pid: u32 = content.trim().parse().unwrap();
    assert_eq!(
        written_pid,
        std::process::id(),
        "PID file should contain current process PID"
    );
}

/// When config_dir is empty, prepare_run resolves to the default platform path.
#[test]
fn test_run_empty_config_dir_uses_default() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_str().unwrap().to_string();
    let (resolved, pid) = prepare_run(&config_dir).unwrap();

    assert_eq!(resolved, tmp.path());
    assert_eq!(pid, std::process::id());

    // Verify PID file was written in the specified config_dir
    let pid_file = tmp.path().join("daemon.pid");
    assert!(
        pid_file.exists(),
        "PID file should exist in specified config_dir"
    );

    let content = fs::read_to_string(&pid_file).unwrap();
    let written_pid: u32 = content.trim().parse().unwrap();
    assert_eq!(
        written_pid,
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
