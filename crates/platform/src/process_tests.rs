use crate::process::{
    check_stale_pid, is_process_alive, pid_file_path, read_pid_file, send_signal, spawn_daemon,
    write_pid_file, SpawnOptions,
};
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use tempfile::TempDir;

// ── is_process_alive boundary tests ──────────────────────────────

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

// ── check_stale_pid boundary tests ───────────────────────────────

/// Alive PID file should NOT be removed by check_stale_pid.
#[test]
fn test_check_stale_pid_alive_preserves_file() {
    let tmp = TempDir::new().unwrap();
    let path = pid_file_path(tmp.path());
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
    let path = pid_file_path(tmp.path());
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
    let path = pid_file_path(tmp.path());
    assert!(!path.exists());

    let result = check_stale_pid(&path).unwrap();
    assert_eq!(result, None);
    assert!(
        !path.exists(),
        "should not create a PID file when none existed"
    );
}

#[test]
fn test_write_and_read_pid_file() {
    let tmp = TempDir::new().unwrap();
    let path = pid_file_path(tmp.path());

    write_pid_file(&path, 12345).unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, Some(12345));
}

#[test]
fn test_read_pid_file_missing() {
    let path = std::path::Path::new("/nonexistent/daemon.pid");
    assert_eq!(read_pid_file(path), None);
}

#[test]
fn test_write_pid_file_overwrite() {
    let tmp = TempDir::new().unwrap();
    let path = pid_file_path(tmp.path());

    write_pid_file(&path, 111).unwrap();
    write_pid_file(&path, 222).unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, Some(222), "should read the latest written PID");
}

#[test]
fn test_write_pid_file_invalid_content() {
    let tmp = TempDir::new().unwrap();
    let path = pid_file_path(tmp.path());

    // Manually write non-numeric content.
    std::fs::write(&path, "not_a_number").unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, None, "non-numeric PID file should return None");
}

#[test]
fn test_pid_file_path_format() {
    let dir = std::path::Path::new("/tmp/test");
    let path = pid_file_path(dir);
    assert_eq!(path, std::path::PathBuf::from("/tmp/test/daemon.pid"));
}

// ── send_signal tests ──────────────────────────────────────────────

/// Helper: spawn a long-running child process and return it.
fn spawn_sleep_child() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("60")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn sleep child")
}

#[cfg(unix)]
#[test]
fn test_send_signal_sigterm() {
    let mut child = spawn_sleep_child();
    let pid = child.id();

    // Send SIGTERM (force=false). Should succeed and terminate the child.
    send_signal(pid, false).expect("send_signal(pid, SIGTERM) failed");
    let status = child.wait().unwrap();
    // Default SIGTERM handler kills with signal 15.
    assert_eq!(
        status.signal(),
        Some(15),
        "child should be killed by SIGTERM: {status}"
    );
}

#[cfg(unix)]
#[test]
fn test_send_signal_sigint() {
    let mut child = spawn_sleep_child();
    let pid = child.id();

    // Send SIGINT (force=true). Should succeed and terminate the child.
    send_signal(pid, true).expect("send_signal(pid, SIGINT) failed");
    let status = child.wait().unwrap();
    // SIGINT = signal 2; default handler terminates the process.
    assert_eq!(
        status.signal(),
        Some(2),
        "child should be killed by SIGINT: {status}"
    );
}

#[test]
fn test_send_signal_invalid_pid() {
    // PID 999999999 is almost certainly not running.
    let err = send_signal(999999999, false);
    assert!(err.is_err(), "send_signal to invalid PID should fail");
}

#[test]
fn test_send_signal_invalid_pid_force() {
    let err = send_signal(999999999, true);
    assert!(
        err.is_err(),
        "send_signal(force) to invalid PID should fail"
    );
}

// ── spawn_daemon tests ────────────────────────────────────────────

#[test]
fn test_spawn_daemon_writes_pid_file() {
    let config_dir = tempfile::tempdir().unwrap();
    let mut child = spawn_daemon(
        "sleep",
        &["60"],
        config_dir.path(),
        &SpawnOptions::default(),
    )
    .expect("spawn_daemon failed");

    let pid = child.id();
    let path = pid_file_path(config_dir.path());
    let stored = read_pid_file(&path);
    assert_eq!(
        stored,
        Some(pid),
        "PID file should contain the spawned child PID"
    );

    // Clean up child process.
    child.kill().ok();
    child.wait().ok();
}

#[test]
fn test_spawn_daemon_invalid_command() {
    let config_dir = tempfile::tempdir().unwrap();
    let result = spawn_daemon(
        "/nonexistent/command",
        &[],
        config_dir.path(),
        &SpawnOptions::default(),
    );
    assert!(
        result.is_err(),
        "spawn_daemon with invalid command should return error"
    );
}

// ── is_process_alive tests ────────────────────────────────────────

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
