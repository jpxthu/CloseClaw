//! Integration test for SIGTERM graceful shutdown
//!
//! Verifies that `closeclaw run --foreground` + SIGTERM/SIGINT
//! triggers graceful shutdown instead of hard-killing the daemon.

use closeclaw_common::test_helpers::write_mandatory_configs;
use std::time::Duration;
use tokio::time::timeout;

use super::helpers;

/// Verifies that SIGTERM triggers graceful shutdown (not hard kill).
/// The daemon should exit with code 0 after drain timeout.
#[tokio::test]
#[cfg(unix)]
async fn test_sigterm_triggers_graceful_shutdown() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_dir = temp_dir.path();

    let agents_dir = config_dir.join("config");
    std::fs::create_dir_all(&agents_dir).expect("create config dir");
    std::fs::write(
        agents_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )
    .expect("failed to write test agents.json");
    write_mandatory_configs(&agents_dir).expect("write mandatory config");

    // Spawn daemon with HOME isolation
    let mut daemon = helpers::spawn_daemon(config_dir);

    // Wait for daemon admin socket to be ready
    helpers::wait_for_daemon_ready(config_dir).await;

    // Verify daemon is still running (not crashed on startup)
    helpers::assert_daemon_alive(&mut daemon);

    let pid = daemon.id().expect("daemon has a PID");

    // Send SIGTERM
    // SAFETY: `pid` is the PID of the daemon child we spawned above and
    // verified is still running; the cast to `libc::pid_t` is a lossless
    // widening conversion, and SIGTERM is a valid signal number.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }

    // Wait for daemon to exit (drain timeout is 30s, add buffer)
    let result = timeout(Duration::from_secs(35), daemon.wait())
        .await
        .expect("daemon should exit within 35s");

    let status = result.expect("daemon should exit");

    assert!(
        status.success(),
        "daemon should exit successfully after graceful shutdown, got: {:?}",
        status
    );
}

/// Verifies that SIGINT (Ctrl+C) also triggers graceful shutdown.
#[tokio::test]
#[cfg(unix)]
async fn test_sigint_triggers_graceful_shutdown() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_dir = temp_dir.path();

    let agents_dir = config_dir.join("config");
    std::fs::create_dir_all(&agents_dir).expect("create config dir");
    std::fs::write(
        agents_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )
    .expect("failed to write test agents.json");
    write_mandatory_configs(&agents_dir).expect("write mandatory config");

    // Spawn daemon with HOME isolation
    let mut daemon = helpers::spawn_daemon(config_dir);

    // Wait for daemon admin socket to be ready
    helpers::wait_for_daemon_ready(config_dir).await;

    // Verify daemon is still running
    helpers::assert_daemon_alive(&mut daemon);

    let pid = daemon.id().expect("daemon has a PID");

    // Send SIGINT
    // SAFETY: `pid` is the PID of the daemon child we spawned above and
    // verified is still running; the cast to `libc::pid_t` is a lossless
    // widening conversion, and SIGINT is a valid signal number.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGINT);
    }

    let result = timeout(Duration::from_secs(35), daemon.wait())
        .await
        .expect("daemon should exit within 35s");

    let status = result.expect("daemon should exit");

    assert!(
        status.success(),
        "daemon should exit successfully after graceful shutdown, got: {:?}",
        status
    );
}
