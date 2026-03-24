//! Integration test for SIGTERM graceful shutdown
//!
//! Verifies that `closeclaw stop` (SIGTERM) triggers graceful shutdown
//! instead of hard-killing the daemon.

use std::time::Duration;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::timeout;

/// Returns the path to the `closeclaw` daemon binary (not the test binary).
fn closeclaw_binary() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("target/debug/closeclaw")
}

/// Verifies that SIGTERM triggers graceful shutdown (not hard kill).
/// The daemon should exit with SIGTERM signal (exit code = 128 + 15 = 143).
#[tokio::test]
#[cfg(unix)]
async fn test_sigterm_triggers_graceful_shutdown() {
    

    // Build the daemon binary path (test binary != closeclaw daemon)
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let daemon_bin = manifest_dir.join("target/debug/closeclaw");

    // Create a temporary config directory with minimal config
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_dir = temp_dir.path();

    // Write a minimal agents.json so daemon doesn't fail to load config
    std::fs::write(
        config_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )
    .expect("failed to write test agents.json");

    // Start the daemon
    let mut daemon = Command::new(&daemon_bin)
        .args(["run", "--config-dir"])
        .arg(config_dir.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn daemon");

    // Wait for daemon to fully initialize
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify daemon is still running (not crashed on startup)
    match daemon.try_wait() {
        Ok(Some(status)) => {
            // Daemon crashed — capture output for diagnosis
            let output = daemon.wait_with_output().await.expect("can capture output");
            panic!(
                "daemon exited prematurely during startup: {:?}\nstdout: {}\nstderr: {}",
                status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(None) => { /* still running, good */ }
        Err(e) => {
            panic!("error checking daemon status: {}", e);
        }
    }

    // Get the PID to send signal
    let pid = daemon.id().expect("daemon has a PID");

    // Send SIGTERM using libc::kill
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }

    // Wait for daemon to exit (drain timeout is 30s, add buffer)
    let result = timeout(Duration::from_secs(35), daemon.wait())
        .await
        .expect("daemon should exit within 35s");

    let status = result.expect("daemon should exit");

    // After graceful shutdown (drain timeout 30s), daemon exits with code 0
    // The key is that SIGTERM triggers graceful shutdown, not immediate kill
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
    let daemon_bin = closeclaw_binary();

    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_dir = temp_dir.path();

    std::fs::write(
        config_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )
    .expect("failed to write test agents.json");

    let mut daemon = Command::new(&daemon_bin)
        .args(["run", "--config-dir"])
        .arg(config_dir.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn daemon");

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify daemon is still running
    match daemon.try_wait() {
        Ok(Some(status)) => {
            let output = daemon.wait_with_output().await.expect("can capture output");
            panic!(
                "daemon exited prematurely during startup: {:?}\nstdout: {}\nstderr: {}",
                status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(None) => { /* still running */ }
        Err(e) => panic!("error checking daemon status: {}", e),
    }

    // Send SIGINT using libc::kill
    let pid = daemon.id().expect("daemon has a PID");
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGINT);
    }

    let result = timeout(Duration::from_secs(35), daemon.wait())
        .await
        .expect("daemon should exit within 35s");

    let status = result.expect("daemon should exit");

    // After graceful shutdown (drain timeout 30s), daemon exits with code 0
    assert!(
        status.success(),
        "daemon should exit successfully after graceful shutdown, got: {:?}",
        status
    );
}
