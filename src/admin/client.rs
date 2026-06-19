//! Admin RPC client — connects to the admin server socket and sends
//! requests.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

use crate::admin::protocol::{AdminRequest, AdminResponse};

/// Default timeout for admin RPC operations (milliseconds).
const ADMIN_TIMEOUT_MS: u64 = 5000;

/// Admin RPC client that connects to the daemon's admin socket.
pub struct AdminClient {
    path: String,
    timeout_ms: u64,
}

impl AdminClient {
    /// Create a new client targeting the given socket path.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            timeout_ms: ADMIN_TIMEOUT_MS,
        }
    }

    /// Create a client with a custom timeout.
    pub fn with_timeout(path: impl Into<String>, timeout_ms: u64) -> Self {
        Self {
            path: path.into(),
            timeout_ms,
        }
    }

    /// Send a request and return the response.
    pub async fn call(&self, request: &AdminRequest) -> std::io::Result<AdminResponse> {
        let stream = timeout(
            Duration::from_millis(self.timeout_ms),
            UnixStream::connect(&self.path),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "admin RPC connect timeout")
        })??;

        let (reader, mut writer) = stream.into_split();

        // Send request
        let json = serde_json::to_vec(request)?;
        let len = (json.len() as u32).to_be_bytes();
        writer.write_all(&len).await?;
        writer.write_all(&json).await?;
        writer.flush().await?;

        // Read response header
        let mut hdr = [0u8; 4];
        let mut reader = BufReader::new(reader);
        reader.read_exact(&mut hdr).await?;
        let body_len = u32::from_be_bytes(hdr) as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await?;

        serde_json::from_slice(&body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Convenience method: ping the server and return true if Pong.
    pub async fn ping(&self) -> bool {
        matches!(
            self.call(&AdminRequest::Ping).await,
            Ok(AdminResponse::Pong)
        )
    }
}

/// Resolve the admin socket path from the config directory.
pub fn admin_socket_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("admin.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_client_new() {
        let client = AdminClient::new("/tmp/test.sock");
        assert_eq!(client.path, "/tmp/test.sock");
        assert_eq!(client.timeout_ms, ADMIN_TIMEOUT_MS);
    }

    #[test]
    fn test_admin_client_with_timeout() {
        let client = AdminClient::with_timeout("/tmp/test.sock", 1000);
        assert_eq!(client.timeout_ms, 1000);
    }

    #[test]
    fn test_admin_socket_path() {
        let path = admin_socket_path(Path::new("/home/user/.closeclaw"));
        assert_eq!(
            path,
            std::path::PathBuf::from("/home/user/.closeclaw/admin.sock")
        );
    }

    #[tokio::test]
    async fn test_client_connect_to_nonexistent_socket() {
        let client = AdminClient::new("/tmp/nonexistent-admin-test.sock");
        let result = client.call(&AdminRequest::Ping).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_client_ping_to_nonexistent_socket() {
        let client = AdminClient::new("/tmp/nonexistent-admin-test.sock");
        assert!(!client.ping().await);
    }
}
