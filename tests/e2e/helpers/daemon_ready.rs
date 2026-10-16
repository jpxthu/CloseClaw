//! Daemon readiness polling helpers.

use std::path::Path;
use std::time::Duration;

use tokio::net::UnixStream as TokioUnixStream;

/// Default timeout for waiting on the daemon admin socket.
const DEFAULT_SOCKET_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Polls the daemon RPC Unix sockets (`admin.sock` and `chat.sock`)
/// until both accept connections or the timeout is exceeded.
///
/// Uses `DEFAULT_SOCKET_WAIT_TIMEOUT` (15s) if no custom timeout is provided.
/// Returns when both sockets are ready; panics if the timeout is exceeded.
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
/// Returns when both sockets are ready; panics if the timeout is exceeded.
pub async fn wait_for_daemon_ready_with_timeout(config_dir: &Path, timeout: Duration) {
    let admin_socket_path = config_dir.join("admin.sock");
    let chat_socket_path = config_dir.join("chat.sock");
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let admin_ready = socket_ready(&admin_socket_path).await;
        let chat_ready = socket_ready(&chat_socket_path).await;
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

/// Single readiness probe: async connect to `path`, capped per attempt
/// by [`SOCKET_POLL_INTERVAL`]. The cap only guards against a hung
/// connect — a fast failure (e.g. ENOENT) must count as not ready,
/// hence the `matches!(.., Ok(Ok(_)))` check on the nested `Result`s.
async fn socket_ready(path: &Path) -> bool {
    matches!(
        tokio::time::timeout(SOCKET_POLL_INTERVAL, TokioUnixStream::connect(path)).await,
        Ok(Ok(_))
    )
}
