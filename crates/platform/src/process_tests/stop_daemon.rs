use super::spawn_sleep_child;
use crate::process::{stop_daemon, wait_for_exit, write_pid_file, StopOutcome};
use tempfile::TempDir;

/// Helper: spawn a detached sleep process reparented to init.
///
/// Re-enters this test binary via `current_exe` with an env-gated child
/// branch (`test_spawn_detached_sleep_pid_child`): the child process
/// spawns sleep, records its PID in the pidfile, then exits without
/// waiting, so the sleep process is reparented to init. When killed,
/// init reaps it — no zombie. Returns the PID of the actual sleep process.
fn spawn_detached_sleep_pid() -> u32 {
    let pid_file = tempfile::NamedTempFile::new().expect("tempfile");
    let pid_path = pid_file.path().to_path_buf();
    let test_binary = std::env::current_exe().expect("current exe");
    let mut child = std::process::Command::new(&test_binary)
        .env("DETACHED_SLEEP_PIDFILE", &pid_path)
        .arg("--exact")
        .arg("process_tests::stop_daemon::test_spawn_detached_sleep_pid_child")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", test_binary.display()));
    // Wait for the PID file to be written by the grandchild. The child is a
    // full libtest harness (not a tiny helper binary), so its startup cost is
    // much higher — allow >=5s before giving up, or slow environments would
    // leave a flaky window.
    for _ in 0..250 {
        if let Ok(content) = std::fs::read_to_string(&pid_path) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                // The child binary exits right after writing the pidfile;
                // reaping it also guarantees the sleep process has been
                // reparented to init before stop_daemon signals it.
                child.wait().expect("failed to wait on child binary");
                return pid;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    child.kill().ok();
    child.wait().ok();
    panic!("detached sleep PID not found in {pid_path:?}");
}

/// Subprocess child: spawns sleep, writes its PID to the pidfile given
/// via `DETACHED_SLEEP_PIDFILE`, then exits without waiting so the sleep
/// process is reparented to init (detach semantics — init reaps it).
#[test]
fn test_spawn_detached_sleep_pid_child() {
    let pid_path = match std::env::var("DETACHED_SLEEP_PIDFILE") {
        Ok(path) => path,
        Err(_) => {
            eprintln!("skipped: run via parent test subprocess");
            return;
        }
    };
    let child = spawn_sleep_child();
    std::fs::write(pid_path, child.id().to_string()).expect("failed to write pid file");
    // Detach: the handle is dropped without waiting; when this child
    // process exits, the sleep process is reparented to init.
    drop(child);
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
#[test]
fn test_stop_daemon_normal() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    let pid = spawn_detached_sleep_pid();
    write_pid_file(&path, pid).unwrap();

    let outcome = stop_daemon(&path, std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(outcome, StopOutcome::Stopped(pid));
    assert!(!path.exists(), "PID file should be removed after stop");
}

/// Timeout path: process frozen with SIGSTOP → Err + PID file preserved.
///
/// The process is stopped (SIGSTOP) so it cannot respond to SIGTERM.
/// is_process_alive still reports it as alive, so the timeout fires.
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
    // SAFETY: `pid` is the test's own spawned child and SIGSTOP merely
    // suspends it; kill(2) with a valid signal is a standard POSIX operation
    // and cannot cause memory unsafety.
    unsafe {
        libc::kill(pid as i32, libc::SIGSTOP);
    }

    let result = stop_daemon(&path, std::time::Duration::from_millis(200));
    assert!(result.is_err(), "timeout should return Err");
    assert!(
        path.exists(),
        "PID file should be preserved when timeout occurs"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("did not exit within"));

    // Clean up: SIGCONT then SIGKILL so the process can be reaped.
    // SAFETY: `pid` is the test's own spawned child; SIGCONT merely resumes
    // it, and kill(2) with a valid signal is a standard POSIX operation with
    // no memory-safety implications.
    unsafe {
        libc::kill(pid as i32, libc::SIGCONT);
    }
    // SAFETY: kill with SIGKILL is a standard POSIX operation.
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    child.wait().ok();
}

/// No PID file → NotRunning.
#[test]
fn test_stop_daemon_no_pid_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");

    let outcome = stop_daemon(&path, std::time::Duration::from_secs(1)).unwrap();
    assert_eq!(outcome, StopOutcome::NotRunning);
}

/// Stale PID file → NotRunning + file cleaned up.
#[test]
fn test_stop_daemon_stale_pid() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    write_pid_file(&path, 99999999).unwrap();
    assert!(path.exists());

    let outcome = stop_daemon(&path, std::time::Duration::from_secs(1)).unwrap();
    assert_eq!(outcome, StopOutcome::NotRunning);
    assert!(!path.exists(), "stale PID file should be removed");
}

/// Invalid PID content → NotRunning + file preserved (not cleaned up).
#[test]
fn test_stop_daemon_invalid_pid_content() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    std::fs::write(&path, "not_a_number").unwrap();

    let outcome = stop_daemon(&path, std::time::Duration::from_secs(1)).unwrap();
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

    let outcome = stop_daemon(&path, std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(
        outcome,
        StopOutcome::NotRunning,
        "exit race should return NotRunning"
    );
    assert!(!path.exists(), "PID file should be cleaned up on exit race");
}

/// Normal stop path regression: polling-based wait_for_exit works.
/// An alive child is signaled, wait returns Ok, PID file is cleaned.
#[test]
fn test_stop_daemon_normal_polling_wait() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");
    let pid = spawn_detached_sleep_pid();
    write_pid_file(&path, pid).unwrap();

    let outcome = stop_daemon(&path, std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(outcome, StopOutcome::Stopped(pid));
    assert!(!path.exists(), "PID file should be removed after stop");
}
