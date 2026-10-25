use super::spawn_sleep_child;
use crate::process::send_signal;
use std::os::unix::process::ExitStatusExt;
use tokio::signal::unix::SignalKind;

// ── send_signal tests ──────────────────────────────────────────────

#[test]
fn test_send_signal_sigterm() {
    let mut child = spawn_sleep_child();
    let pid = child.id();

    // Send SIGTERM. Should succeed and terminate the child.
    send_signal(pid, SignalKind::terminate()).expect("send_signal(pid, SIGTERM) failed");
    let status = child.wait().unwrap();
    // Default SIGTERM handler kills with signal 15.
    assert_eq!(
        status.signal(),
        Some(15),
        "child should be killed by SIGTERM: {status}"
    );
}
#[test]
fn test_send_signal_sigint() {
    let mut child = spawn_sleep_child();
    let pid = child.id();

    // Send SIGINT. Should succeed and terminate the child.
    send_signal(pid, SignalKind::interrupt()).expect("send_signal(pid, SIGINT) failed");
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
    let err = send_signal(u32::MAX, SignalKind::terminate());
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
    let err = send_signal(999999999, SignalKind::terminate());
    assert!(err.is_err(), "send_signal to invalid PID should fail");
}
