//! Unit tests for prepare_run.
//!
//! Covers config_dir resolution and PID file writing without starting
//! a real daemon.

use crate::cli::admin::config_dir_for;
use crate::cli::admin::run::prepare_run;
use crate::cli::admin::RunOutput;
use std::fs;
use tempfile::TempDir;

/// When a non-empty config_dir is passed, prepare_run resolves to that path;
/// when empty it falls back to the default platform path.
#[test]
fn test_run_config_dir_uses_platform_default() {
    // Case 1: non-empty config_dir → use provided path
    let tmp1 = TempDir::new().unwrap();
    let fake_home = tmp1.path();
    let expected_default = config_dir_for(fake_home);
    let config_dir = expected_default.to_str().unwrap().to_string();
    let (resolved, pid, pid_file) = prepare_run(&config_dir).unwrap();
    assert_eq!(resolved, expected_default);
    assert_eq!(pid, std::process::id());
    assert!(pid_file.exists(), "PID file should exist in config_dir");
    let content = fs::read_to_string(&pid_file).unwrap();
    let written_pid: u32 = content.trim().parse().unwrap();
    assert_eq!(written_pid, std::process::id());

    // Case 2: non-empty config_dir → use provided path
    let tmp2 = TempDir::new().unwrap();
    let empty_dir = tmp2.path().to_str().unwrap().to_string();
    let (resolved2, pid2, pid_file2) = prepare_run(&empty_dir).unwrap();
    assert_eq!(resolved2, tmp2.path());
    assert_eq!(pid2, std::process::id());
    assert!(
        pid_file2.exists(),
        "PID file should exist in resolved config_dir"
    );
    let content2 = fs::read_to_string(&pid_file2).unwrap();
    let written_pid2: u32 = content2.trim().parse().unwrap();
    assert_eq!(written_pid2, std::process::id());
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
