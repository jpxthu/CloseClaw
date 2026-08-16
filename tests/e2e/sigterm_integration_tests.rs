//! Integration test for SIGTERM graceful shutdown
//!
//! Verifies that `closeclaw run --foreground` + SIGTERM/SIGINT
//! triggers graceful shutdown instead of hard-killing the daemon.

use closeclaw_common::test_helpers::write_mandatory_configs;
use std::os::unix::net::UnixStream;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const SOCKET_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Returns the path to the `closeclaw` daemon binary (not the test binary).
fn closeclaw_binary() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("target/debug/closeclaw")
}

/// Polls the admin RPC Unix socket until it accepts connections or times out.
/// Returns Ok(()) when the socket is ready, Err if timeout is exceeded.
async fn wait_for_daemon_ready(config_dir: &std::path::Path) {
    let socket_path = config_dir.join("admin.sock");
    let deadline = tokio::time::Instant::now() + SOCKET_WAIT_TIMEOUT;

    loop {
        if UnixStream::connect(&socket_path).is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "daemon admin socket not ready after {:?}: {}",
                SOCKET_WAIT_TIMEOUT,
                socket_path.display()
            );
        }
        tokio::time::sleep(SOCKET_POLL_INTERVAL).await;
    }
}

/// Verifies that SIGTERM triggers graceful shutdown (not hard kill).
/// The daemon should exit with code 0 after drain timeout.
#[tokio::test]
#[cfg(unix)]
async fn test_sigterm_triggers_graceful_shutdown() {
    let daemon_bin = closeclaw_binary();

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

    // Start daemon in --foreground mode so test process owns the daemon PID
    let mut daemon = Command::new(&daemon_bin)
        .args(["run", "--config-dir"])
        .arg(config_dir.as_os_str())
        .arg("--foreground")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn daemon");

    // Wait for daemon admin socket to be ready
    wait_for_daemon_ready(config_dir).await;

    // Verify daemon is still running (not crashed on startup)
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
        Ok(None) => { /* still running, good */ }
        Err(e) => {
            panic!("error checking daemon status: {}", e);
        }
    }

    let pid = daemon.id().expect("daemon has a PID");

    // Send SIGTERM
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
    let daemon_bin = closeclaw_binary();

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

    // Start daemon in --foreground mode so test process owns the daemon PID
    let mut daemon = Command::new(&daemon_bin)
        .args(["run", "--config-dir"])
        .arg(config_dir.as_os_str())
        .arg("--foreground")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn daemon");

    // Wait for daemon admin socket to be ready
    wait_for_daemon_ready(config_dir).await;

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

    let pid = daemon.id().expect("daemon has a PID");

    // Send SIGINT
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
