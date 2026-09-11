//! Shared helper utilities for e2e tests.
//!
//! Centralizes daemon spawn logic, readiness polling, and lifecycle
//! assertions to avoid duplication across `sigterm_tests`,
//! `agent_profile_tests`, and `shutdown_checkpoint_tests`.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};

/// Default timeout for waiting on the daemon admin socket.
const DEFAULT_SOCKET_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Returns the path to the `closeclaw` daemon binary (not the test binary).
pub fn closeclaw_binary() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("target/debug/closeclaw")
}

/// Polls the admin RPC Unix socket until it accepts connections or times out.
///
/// Uses `DEFAULT_SOCKET_WAIT_TIMEOUT` (15s) if no custom timeout is provided.
/// Returns Ok(()) when the socket is ready, panics if timeout is exceeded.
pub async fn wait_for_daemon_ready(config_dir: &Path) {
    wait_for_daemon_ready_with_timeout(config_dir, DEFAULT_SOCKET_WAIT_TIMEOUT).await;
}

/// Polls the admin RPC Unix socket until it accepts connections or times out.
///
/// `timeout` specifies the maximum duration to wait for the socket to become ready.
/// Returns Ok(()) when the socket is ready, panics if timeout is exceeded.
pub async fn wait_for_daemon_ready_with_timeout(config_dir: &Path, timeout: Duration) {
    let socket_path = config_dir.join("admin.sock");
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if UnixStream::connect(&socket_path).is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "daemon admin socket not ready after {:?}: {}",
                timeout,
                socket_path.display()
            );
        }
        tokio::time::sleep(SOCKET_POLL_INTERVAL).await;
    }
}

/// Spawn the closeclaw daemon in `--foreground` mode with HOME isolation.
///
/// Sets `HOME` to `config_root` so that `pid_file_path()` resolves to
/// `<config_root>/.closeclaw/daemon.pid`, isolating each test from the
/// global `~/.closeclaw/daemon.pid`.
///
/// Returns the `Child` handle. The caller is responsible for lifecycle
/// management (signal + wait). The daemon is spawned with `kill_on_drop`
/// to prevent residual processes on panic paths.
pub fn spawn_daemon(config_root: &Path) -> Child {
    // Ensure .closeclaw dir exists under temp HOME so PID file can be written
    std::fs::create_dir_all(config_root.join(".closeclaw")).expect("create .closeclaw dir");

    Command::new(closeclaw_binary())
        .args(["run", "--config-dir"])
        .arg(config_root.as_os_str())
        .arg("--foreground")
        .env("HOME", config_root.as_os_str())
        .current_dir(config_root.join("config"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn daemon")
}

/// Assert that the daemon process is still alive (has not exited prematurely).
///
/// On failure, includes the exit status details and a hint to check daemon logs.
pub fn assert_daemon_alive(daemon: &mut Child) {
    if let Some(status) = daemon.try_wait().expect("try_wait daemon") {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        panic!(
            "daemon exited prematurely with status: {:?} (exit code: {}). \
             Check daemon stdout/stderr logs for details.",
            status, code
        );
    }
}
