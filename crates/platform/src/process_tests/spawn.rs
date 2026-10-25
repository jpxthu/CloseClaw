use crate::process::{read_pid_file, spawn_daemon, SpawnOptions};
use tempfile::TempDir;

// ── spawn_daemon tests ────────────────────────────────────────────

/// Default (no injection) semantics: `pid_file_path` is `None`, so
/// `spawn_daemon` falls back to the fixed [`pid_file_path`] resolution.
#[test]
fn test_spawn_options_default_pid_file_path_is_none() {
    let options = SpawnOptions::default();
    assert!(
        options.pid_file_path.is_none(),
        "default SpawnOptions must not inject a PID file path"
    );
}

#[test]
fn test_spawn_daemon_writes_pid_file() {
    let tmp = TempDir::new().unwrap();
    // Same shape as the fixed layout: `{root}/.closeclaw/daemon.pid`.
    let path = tmp.path().join(".closeclaw").join("daemon.pid");
    let options = SpawnOptions {
        pid_file_path: Some(path.clone()),
        ..SpawnOptions::default()
    };
    let mut child = spawn_daemon("sleep", &["60"], &options).expect("spawn_daemon failed");

    let pid = child.id();
    let stored = read_pid_file(&path);
    assert_eq!(
        stored,
        Some(pid),
        "PID file should contain the spawned child PID"
    );

    // Clean up: child process; PID file lives inside the TempDir and is
    // removed with it (explicit remove also exercises the cleanup path).
    child.kill().ok();
    child.wait().ok();
    std::fs::remove_file(&path).ok();
}

#[test]
fn test_spawn_daemon_invalid_command() {
    let result = spawn_daemon("/nonexistent/command", &[], &SpawnOptions::default());
    assert!(
        result.is_err(),
        "spawn_daemon with invalid command should return error"
    );
}
