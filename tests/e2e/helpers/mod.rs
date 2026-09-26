//! Shared helper utilities for e2e tests.
//!
//! Centralizes daemon spawn logic, readiness polling, and lifecycle
//! assertions to avoid duplication across `sigterm_tests`,
//! `agent_profile_tests`, `shutdown_checkpoint_tests`, and
//! `gateway_restart_turn_tests`. The minimal config-tree scaffolding
//! (empty `agents.json` + mandatory configs) lives in [`config_tree`] and
//! is **not** feature-gated, so cases built without `fake-llm` can use it.
//! The Chat-RPC client, fake-LLM server, and full fake-LLM config-tree
//! scaffolding live in the [`chat`], [`fake_llm`], and [`config`]
//! submodules (feature `fake-llm`, whose consumers are the fake-LLM-backed
//! test files).

pub mod config_tree;

#[cfg(feature = "fake-llm")]
pub mod chat;
#[cfg(feature = "fake-llm")]
pub mod config;
#[cfg(feature = "fake-llm")]
pub mod fake_llm;

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};

/// Default timeout for waiting on the daemon admin socket.
const DEFAULT_SOCKET_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Upper bound for graceful shutdown after SIGTERM (drain timeout 30s + margin).
#[cfg(feature = "fake-llm")]
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(40);

/// Returns the path to the `closeclaw` daemon binary (not the test binary).
pub fn closeclaw_binary() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("target/debug/closeclaw")
}

/// Polls the daemon RPC Unix sockets (`admin.sock` and `chat.sock`)
/// until both accept connections or the timeout is exceeded.
///
/// Uses `DEFAULT_SOCKET_WAIT_TIMEOUT` (15s) if no custom timeout is provided.
/// Returns Ok(()) when both sockets are ready, panics if timeout is exceeded.
pub async fn wait_for_daemon_ready(config_dir: &Path) {
    wait_for_daemon_ready_with_timeout(config_dir, DEFAULT_SOCKET_WAIT_TIMEOUT).await;
}

/// Polls the daemon RPC Unix sockets (`admin.sock` and `chat.sock`)
/// until both accept connections or the timeout is exceeded.
///
/// The daemon binds `admin.sock` before `chat.sock` (startup phase 6),
/// so a test that only waited on `admin.sock` could proceed into the
/// chat path before `chat.sock` existed (ENOENT race). Both sockets
/// share one loop and one `timeout` deadline.
///
/// Returns Ok(()) when both sockets are ready, panics if timeout is exceeded.
pub async fn wait_for_daemon_ready_with_timeout(config_dir: &Path, timeout: Duration) {
    let admin_socket_path = config_dir.join("admin.sock");
    let chat_socket_path = config_dir.join("chat.sock");
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let admin_ready = UnixStream::connect(&admin_socket_path).is_ok();
        let chat_ready = UnixStream::connect(&chat_socket_path).is_ok();
        if admin_ready && chat_ready {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "daemon sockets not ready after {:?} (admin_ready={}, chat_ready={}): {} / {}",
                timeout,
                admin_ready,
                chat_ready,
                admin_socket_path.display(),
                chat_socket_path.display()
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
    assert_daemon_alive_with_context(daemon, None);
}

/// [`assert_daemon_alive`] with an optional caller-supplied context
/// phrase folded into the panic message (e.g. `Some("during gateway
/// restart")`) so failures read where in the scenario the daemon died.
pub fn assert_daemon_alive_with_context(daemon: &mut Child, context: Option<&str>) {
    if let Some(status) = daemon.try_wait().expect("try_wait daemon") {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        let context = context.map(|c| format!(" {c}")).unwrap_or_default();
        panic!(
            "daemon exited prematurely{context} with status: {:?} (exit code: {}). \
             Check daemon stdout/stderr logs for details.",
            status, code
        );
    }
}

/// Send SIGTERM to `daemon` and wait for its exit within
/// [`SHUTDOWN_TIMEOUT`]. Returns the exit status — the caller asserts
/// on it. Shared by `agent_profile_tests` (DaemonGuard::shutdown) and
/// `gateway_restart_turn_tests` (terminate_daemon), Step 1.23.
#[cfg(feature = "fake-llm")]
pub async fn sigterm_and_wait(daemon: &mut Child) -> std::process::ExitStatus {
    let pid = daemon.id().expect("daemon has a PID") as libc::pid_t;
    // SAFETY: `pid` is the PID of the daemon child this caller spawned
    // and holds; the cast to `libc::pid_t` is a lossless widening
    // conversion, and SIGTERM is a valid signal number.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    tokio::time::timeout(SHUTDOWN_TIMEOUT, daemon.wait())
        .await
        .expect("daemon should exit within the shutdown timeout")
        .expect("daemon exit status should be observable")
}
