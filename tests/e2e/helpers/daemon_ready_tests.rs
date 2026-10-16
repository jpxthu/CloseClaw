//! Contract tests for [`super::daemon_ready::wait_for_daemon_ready_with_timeout`].

use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::Duration;

use super::daemon_ready::wait_for_daemon_ready_with_timeout;

/// Short timeout for the panic paths: fast failure, no sleep coincidence
/// (STANDARDS.md §6 — keep test waits far below the 1s bar).
const SHORT_TIMEOUT: Duration = Duration::from_millis(100);

/// Binds a real Unix socket at `<dir>/<name>` (the daemon's admin/chat
/// socket path shape). The returned listener must stay alive so the
/// polled `connect` keeps succeeding.
fn bind_socket(dir: &Path, name: &str) -> UnixListener {
    UnixListener::bind(dir.join(name)).expect("bind Unix socket")
}

/// Normal path: both `admin.sock` and `chat.sock` are bound — the wait
/// returns (without panicking) instead of polling to the timeout.
#[tokio::test]
async fn test_wait_for_daemon_ready_returns_when_both_sockets_bound() {
    let dir = tempfile::TempDir::new().expect("create temp dir under system temp");
    let _admin = bind_socket(dir.path(), "admin.sock");
    let _chat = bind_socket(dir.path(), "chat.sock");

    wait_for_daemon_ready_with_timeout(dir.path(), SHORT_TIMEOUT).await;
}

/// Error path: neither socket exists — the wait polls until the deadline
/// and panics with the timeout diagnostics.
#[tokio::test]
#[should_panic(expected = "not ready after 100ms")]
async fn test_wait_for_daemon_ready_panics_when_sockets_missing() {
    let dir = tempfile::TempDir::new().expect("create temp dir under system temp");

    wait_for_daemon_ready_with_timeout(dir.path(), SHORT_TIMEOUT).await;
}

/// Boundary: only `admin.sock` is bound — the wait must NOT return on a
/// single ready socket; it keeps polling and panics at the deadline,
/// reporting exactly which socket is still not ready.
#[tokio::test]
#[should_panic(expected = "(admin_ready=true, chat_ready=false)")]
async fn test_wait_for_daemon_ready_polls_until_timeout_when_only_admin_bound() {
    let dir = tempfile::TempDir::new().expect("create temp dir under system temp");
    let _admin = bind_socket(dir.path(), "admin.sock");

    wait_for_daemon_ready_with_timeout(dir.path(), SHORT_TIMEOUT).await;
}
