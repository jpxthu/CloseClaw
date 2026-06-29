//! Unit tests for prepare_run and run helpers.
//!
//! Covers config_dir resolution and PID file writing without starting
//! a real daemon.

use crate::cli::admin::config_dir_for;
use crate::cli::admin::run::prepare_run;
use crate::cli::admin::RunOutput;
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
