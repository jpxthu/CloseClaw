use crate::platform::process::{pid_file_path, read_pid_file, send_signal, write_pid_file};
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use tempfile::TempDir;

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
fn test_send_signal_sigkill() {
    let mut child = spawn_sleep_child();
    let pid = child.id();

    // Send SIGKILL (force=true). Should succeed and terminate the child.
    send_signal(pid, true).expect("send_signal(pid, SIGKILL) failed");
    let status = child.wait().unwrap();
    // SIGKILL cannot be caught; process must exit with signal 9.
    assert_eq!(
        status.signal(),
        Some(9),
        "child should be killed by SIGKILL: {status}"
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
