use crate::process::{
    check_stale_pid, is_process_alive, pid_file_path, pid_file_path_inner, read_pid_file,
    send_signal, spawn_daemon, stop_daemon, wait_for_exit, write_pid_file, SpawnOptions,
    StopOutcome,
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

#[test]
fn test_write_and_read_pid_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");

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
    let path = tmp.path().join("daemon.pid");

    write_pid_file(&path, 111).unwrap();
    write_pid_file(&path, 222).unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, Some(222), "should read the latest written PID");
}

#[test]
fn test_write_pid_file_invalid_content() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");

    // Manually write non-numeric content.
    std::fs::write(&path, "not_a_number").unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, None, "non-numeric PID file should return None");
}

#[test]
fn test_pid_file_path_inner() {
    let path = pid_file_path_inner("/home/user");
    assert_eq!(
        path,
        std::path::PathBuf::from("/home/user/.closeclaw/daemon.pid")
    );
}

#[test]
fn test_pid_file_path_inner_empty_home() {
    let path = pid_file_path_inner("");
    assert_eq!(path, std::path::PathBuf::from(".closeclaw/daemon.pid"));
}

// ── pid_file_path_inner pure computation tests ───────────────────────

/// pid_file_path_inner is pure path computation — no I/O side effects.
/// Running it twice with the same input must produce identical results,
/// and the path must NOT refer to a file that was created as a side effect.
#[test]
fn test_pid_file_path_inner_is_pure_no_io() {
    let path1 = pid_file_path_inner("/some/home");
    let path2 = pid_file_path_inner("/some/home");
    assert_eq!(path1, path2, "pure function must be idempotent");
    assert!(
        !path1.exists(),
        "pure function must not create files on disk"
    );
}

// ── pid_file_path() consistency tests ───────────────────────────────

/// pid_file_path() must return the same result as pid_file_path_inner(HOME)
/// — the public API delegates to the injectable inner, forming a single
/// source of truth for the fixed PID path.
#[test]
fn test_pid_file_path_matches_inner_with_home() {
    let public = pid_file_path().unwrap();
    let home = std::env::var("HOME").unwrap();
    let inner = pid_file_path_inner(&home);
    assert_eq!(
        public, inner,
        "pid_file_path() must equal pid_file_path_inner(HOME)"
    );
    assert!(public.is_absolute(), "fixed PID path must be absolute");
    assert!(public.to_string_lossy().ends_with("daemon.pid"));
}

/// pid_file_path() is a no-arg constant — multiple calls must always
/// return the same value, regardless of when or how often called.
#[test]
fn test_pid_file_path_is_stable_constant() {
    let first = pid_file_path().unwrap();
    let second = pid_file_path().unwrap();
    let third = pid_file_path().unwrap();
    assert_eq!(first, second);
    assert_eq!(second, third);
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

/// Helper: spawn a detached sleep process reparented to init.
///
/// Uses a double-fork helper binary so the grandchild (sleep) is
/// reparented to init. When killed, init reaps it — no zombie.
/// Returns the PID of the actual sleep process.
#[cfg(unix)]
fn spawn_detached_sleep_pid() -> u32 {
    let pid_file = tempfile::NamedTempFile::new().expect("tempfile");
    let pid_path = pid_file.path().to_path_buf();
    // Run helper in background (non-blocking) so it doesn't hang on pipe.
    std::process::Command::new("/tmp/detach_helper")
        .arg(pid_path.to_str().unwrap())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn /tmp/detach_helper");
    // Wait for the PID file to be written by the grandchild.
    for _ in 0..50 {
        if let Ok(content) = std::fs::read_to_string(&pid_path) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                return pid;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("detached sleep PID not found in {pid_path:?}");
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

/// PID exceeding i32::MAX must fail with overflow error, not cast to negative.
#[test]
fn test_send_signal_pid_overflow() {
    let err = send_signal(u32::MAX, false);
    assert!(
        err.is_err(),
        "send_signal with pid > i32::MAX should return Err"
    );
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("PID exceeds i32::MAX"),
        "error should mention overflow: {}",
        msg
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
    let mut child =
        spawn_daemon("sleep", &["60"], &SpawnOptions::default()).expect("spawn_daemon failed");

    let pid = child.id();
    let home = std::env::var("HOME").unwrap();
    let path = pid_file_path_inner(&home);
    let stored = read_pid_file(&path);
    assert_eq!(
        stored,
        Some(pid),
        "PID file should contain the spawned child PID"
    );

    // Clean up: child process + PID file left at the fixed path.
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

// ── wait_for_exit boundary tests ──────────────────────────────────

/// Dead process (killed+waited) should return Ok immediately.
#[test]
fn test_wait_for_exit_dead_process() {
    let mut child = spawn_sleep_child();
    let pid = child.id();
    child.kill().expect("failed to kill child");
    child.wait().expect("failed to wait on child");

    let result = wait_for_exit(pid, std::time::Duration::from_secs(1));
    assert!(
        result.is_ok(),
        "wait_for_exit on dead process should return Ok: {:?}",
        result
    );
}

/// Alive process with very short timeout should return Err, not panic.
#[test]
fn test_wait_for_exit_alive_short_timeout() {
    let mut child = spawn_sleep_child();
    let pid = child.id();

    let result = wait_for_exit(pid, std::time::Duration::from_millis(200));
    assert!(
        result.is_err(),
        "wait_for_exit on alive process with short timeout should return Err"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("did not exit within"),
        "error should mention timeout: {}",
        err_msg
    );

    child.kill().ok();
    child.wait().ok();
}

/// PID 1 (init/systemd) + short timeout should return Err.
#[test]
fn test_wait_for_exit_pid1_short_timeout() {
    let result = wait_for_exit(1, std::time::Duration::from_millis(200));
    assert!(
        result.is_err(),
        "wait_for_exit on PID 1 with short timeout should return Err"
    );
}

/// Dead PID (never existed) should return Ok immediately.
#[test]
fn test_wait_for_exit_nonexistent_pid() {
    let result = wait_for_exit(99999999, std::time::Duration::from_secs(1));
    assert!(
        result.is_ok(),
        "wait_for_exit on non-existent PID should return Ok: {:?}",
        result
    );
}

// ── stop_daemon tests ──────────────────────────────────────────────

/// Normal path: alive process is stopped and PID file is cleaned up.
#[cfg(unix)]
#[test]
fn test_stop_daemon_normal() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    let pid = spawn_detached_sleep_pid();
    write_pid_file(&path, pid).unwrap();

    let outcome = stop_daemon(&path, false, std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(outcome, StopOutcome::Stopped(pid));
    assert!(!path.exists(), "PID file should be removed after stop");
}

/// Normal path with force (SIGINT): alive process is stopped and PID file is cleaned up.
#[cfg(unix)]
#[test]
fn test_stop_daemon_normal_force() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    let pid = spawn_detached_sleep_pid();
    write_pid_file(&path, pid).unwrap();

    let outcome = stop_daemon(&path, true, std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(outcome, StopOutcome::Stopped(pid));
    assert!(!path.exists(), "PID file should be removed after stop");
}

/// Timeout path: process frozen with SIGSTOP → Err + PID file preserved.
///
/// The process is stopped (SIGSTOP) so it cannot respond to SIGTERM.
/// is_process_alive still reports it as alive, so the timeout fires.
#[cfg(unix)]
#[test]
fn test_stop_daemon_timeout() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    // Spawn sleep, then SIGSTOP it so it cannot exit.
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn sleep child");
    let pid = child.id();
    write_pid_file(&path, pid).unwrap();
    // Freeze the process so it cannot respond to SIGTERM.
    unsafe {
        libc::kill(pid as i32, libc::SIGSTOP);
    }

    let result = stop_daemon(&path, false, std::time::Duration::from_millis(200));
    assert!(result.is_err(), "timeout should return Err");
    assert!(
        path.exists(),
        "PID file should be preserved when timeout occurs"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("did not exit within"));

    // Clean up: SIGCONT then SIGKILL so the process can be reaped.
    unsafe {
        libc::kill(pid as i32, libc::SIGCONT);
    }
    send_signal(pid, true).ok();
    child.wait().ok();
}

/// No PID file → NotRunning.
#[test]
fn test_stop_daemon_no_pid_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");

    let outcome = stop_daemon(&path, false, std::time::Duration::from_secs(1)).unwrap();
    assert_eq!(outcome, StopOutcome::NotRunning);
}

/// Stale PID file → NotRunning + file cleaned up.
#[test]
fn test_stop_daemon_stale_pid() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    write_pid_file(&path, 99999999).unwrap();
    assert!(path.exists());

    let outcome = stop_daemon(&path, false, std::time::Duration::from_secs(1)).unwrap();
    assert_eq!(outcome, StopOutcome::NotRunning);
    assert!(!path.exists(), "stale PID file should be removed");
}

/// Invalid PID content → NotRunning + file preserved (not cleaned up).
#[test]
fn test_stop_daemon_invalid_pid_content() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    std::fs::write(&path, "not_a_number").unwrap();

    let outcome = stop_daemon(&path, false, std::time::Duration::from_secs(1)).unwrap();
    assert_eq!(outcome, StopOutcome::NotRunning);
    // stop_daemon cleans up the PID file only when it reads a valid PID
    // and the process is not alive. Invalid (non-numeric) content yields
    // None from read_pid_file, so the file is preserved as-as.
    assert!(path.exists(), "invalid PID file should be preserved");
}

// ── Step 1.6: exit race and polling-wait regression tests ────────

/// Exit race: send_signal fails because process already exited.
/// stop_daemon should return NotRunning and clean up the PID file.
#[test]
fn test_stop_daemon_exit_race() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    // Spawn a child, kill it, and reap it — PID is now dead.
    let mut child = spawn_sleep_child();
    let pid = child.id();
    child.kill().expect("failed to kill child");
    child.wait().expect("failed to wait on child");
    // Write PID file *after* the process is dead (simulating race).
    write_pid_file(&path, pid).unwrap();

    let outcome = stop_daemon(&path, false, std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(
        outcome,
        StopOutcome::NotRunning,
        "exit race should return NotRunning"
    );
    assert!(!path.exists(), "PID file should be cleaned up on exit race");
}

/// Normal stop path regression: polling-based wait_for_exit works.
/// An alive child is signaled, wait returns Ok, PID file is cleaned.
#[cfg(unix)]
#[test]
fn test_stop_daemon_normal_polling_wait() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    let pid = spawn_detached_sleep_pid();
    write_pid_file(&path, pid).unwrap();

    let outcome = stop_daemon(&path, false, std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(outcome, StopOutcome::Stopped(pid));
    assert!(!path.exists(), "PID file should be removed after stop");
}
