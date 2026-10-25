use super::spawn_sleep_child;
use crate::process::{check_stale_pid, is_process_alive, write_pid_file};
use tempfile::TempDir;

// ── is_process_alive tests ────────────────────────────────────────

/// PID 1 (init/systemd) should be alive on any running Linux system.
#[test]
fn test_is_process_alive_pid1() {
    assert!(
        is_process_alive(1),
        "PID 1 (init/systemd) should be alive on a running system"
    );
}

/// An extremely large PID (u32::MAX) should not be alive.
#[test]
fn test_is_process_alive_large_pid() {
    assert!(
        !is_process_alive(u32::MAX),
        "PID u32::MAX should not be alive"
    );
}

/// A killed child process should no longer be alive after wait.
#[test]
fn test_is_process_alive_after_kill() {
    let mut child = spawn_sleep_child();
    let pid = child.id();
    assert!(is_process_alive(pid), "child should be alive before kill");
    child.kill().expect("failed to kill child");
    child.wait().expect("failed to wait on child");
    // After wait, the PID is reaped by the OS.
    assert!(
        !is_process_alive(pid),
        "killed+waited child should not be alive"
    );
}

#[test]
fn test_is_process_alive_self() {
    // The current process is definitely alive.
    let pid = std::process::id();
    assert!(is_process_alive(pid), "current process should be alive");
}

#[test]
fn test_is_process_alive_nonexistent() {
    // PID 99999999 almost certainly does not exist.
    assert!(
        !is_process_alive(99999999),
        "non-existent PID should not be alive"
    );
}

// ── check_stale_pid boundary tests ───────────────────────────────

/// Alive PID file should NOT be removed by check_stale_pid.
#[test]
fn test_check_stale_pid_alive_preserves_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    let my_pid = std::process::id();
    write_pid_file(&path, my_pid).unwrap();

    let result = check_stale_pid(&path).unwrap();
    assert_eq!(result, Some(my_pid));
    assert!(
        path.exists(),
        "PID file must be preserved for alive process"
    );
}

/// Stale PID file should be removed by check_stale_pid.
#[test]
fn test_check_stale_pid_stale_removes_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    write_pid_file(&path, 99999999).unwrap();
    assert!(path.exists(), "PID file should exist before check");

    let result = check_stale_pid(&path).unwrap();
    assert_eq!(result, None, "stale PID should return None");
    assert!(!path.exists(), "stale PID file should be removed");
}

/// No PID file should return None without creating any file.
#[test]
fn test_check_stale_pid_no_file_no_side_effect() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    assert!(!path.exists());

    let result = check_stale_pid(&path).unwrap();
    assert_eq!(result, None);
    assert!(
        !path.exists(),
        "should not create a PID file when none existed"
    );
}

/// Stale PID file already removed by a concurrent cleaner (ENOENT)
/// should be tolerated — treated as successfully cleaned up.
#[test]
fn test_check_stale_pid_enoent_on_remove_tolerated() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    // Write a stale PID (non-existent process).
    write_pid_file(&path, 99999999).unwrap();
    assert!(path.exists());
    // Simulate concurrent removal: delete the file before check_stale_pid.
    std::fs::remove_file(&path).unwrap();
    assert!(!path.exists());

    // check_stale_pid should tolerate ENOENT and return Ok(None).
    let result = check_stale_pid(&path).unwrap();
    assert_eq!(
        result, None,
        "concurrent removal (ENOENT) should return None"
    );
}

/// Non-ENOENT removal failure (e.g. permission denied) must propagate
/// as an error — only NotFound is tolerated.
///
/// We test this by using a path where the parent directory is read-only,
/// causing remove_file to fail with a non-ENOENT error (EACCES).
#[test]
fn test_check_stale_pid_non_enoent_error_propagates() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    write_pid_file(&path, 99999999).unwrap();

    // Remove write permission from the parent directory so remove_file
    // on the child fails with EACCES (not ENOENT).
    let parent = tmp.path();
    let ro = std::fs::Permissions::from_mode(0o555);
    std::fs::set_permissions(parent, ro).unwrap();

    let err = check_stale_pid(&path).unwrap_err();

    // Restore permissions for TempDir cleanup.
    let rw = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(parent, rw).unwrap();

    let msg = err.to_string();
    assert!(
        msg.contains("Permission denied") || msg.contains("Read-only file system"),
        "non-ENOENT error should propagate, got: {}",
        msg
    );
}
