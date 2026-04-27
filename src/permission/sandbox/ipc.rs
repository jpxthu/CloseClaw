//! IPC channel for communication with the sandboxed Permission Engine.
//!
//! Uses length-prefixed JSON frames over a Unix domain socket:
//!
//! ```text
//! [4-byte big-endian length (u32)][JSON frame bytes]
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

use crate::permission::{PermissionEngine, PermissionRequest, PermissionResponse, RuleSet};

// ---------------------------------------------------------------------------
// IPC Constants
// ---------------------------------------------------------------------------

/// Maximum time to wait for a response from the engine over IPC.
pub const IPC_TIMEOUT_MS: u64 = 3000;

// ---------------------------------------------------------------------------
// IPC Messages
// ---------------------------------------------------------------------------

/// Request sent from the host process to the sandboxed engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SandboxRequest {
    /// Ask the engine to evaluate a permission request.
    Evaluate { request: PermissionRequest },
    /// Ask the engine to reload its ruleset.
    ReloadRules { rules: RuleSet },
    /// Ping — returns Pong.
    Ping,
}

/// Response sent from the sandboxed engine back to the host.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SandboxResponse {
    /// Permission evaluation result.
    PermissionResponse(PermissionResponse),
    /// Acknowledge a rules reload.
    RulesReloaded,
    /// Pong acknowledgement.
    Pong,
    /// Engine-side error.
    Error { message: String },
}

// ---------------------------------------------------------------------------
// IPC Channel
// ---------------------------------------------------------------------------

/// Bidirectional IPC channel over a Unix domain socket.
///
/// The protocol uses length-prefixed JSON frames:
///
/// ```text
/// [4-byte big-endian length (u32)][JSON frame bytes]
/// ```
pub struct IpcChannel {
    path: PathBuf,
}

impl IpcChannel {
    /// Create a new IPC channel at the given socket path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Remove the socket file if it already exists (idempotent).
    pub fn clean_up(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Connect to the engine and send a request, returning the response.
    pub async fn call(&self, request: &SandboxRequest) -> std::io::Result<SandboxResponse> {
        let stream = timeout(
            Duration::from_millis(IPC_TIMEOUT_MS),
            UnixStream::connect(&self.path),
        )
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "IPC connect timeout"))??;

        let (reader, mut writer): (_, OwnedWriteHalf) = stream.into_split();

        // Send request: [4-byte len][json]
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

    /// Start listening for connections and dispatch them to the engine.
    ///
    /// Blocks forever, processing each connection in a spawned task.
    pub async fn serve(self, engine: Arc<PermissionEngine>) -> std::io::Result<()> {
        self.clean_up();

        let listener = UnixListener::bind(&self.path)?;

        tracing::info!("engine IPC server listening on {}", self.path.display());

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let engine = engine.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, engine).await {
                            tracing::error!("IPC connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("UnixListener accept error: {}", e);
                }
            }
        }
    }
}

/// Handle a single IPC connection from the host.
async fn handle_connection(
    stream: UnixStream,
    engine: Arc<PermissionEngine>,
) -> std::io::Result<()> {
    let (reader, writer): (_, OwnedWriteHalf) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = writer;

    loop {
        // Read 4-byte length header
        let mut hdr = [0u8; 4];
        match reader.read_exact(&mut hdr).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let body_len = u32::from_be_bytes(hdr) as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await?;

        // Deserialize request
        let request: SandboxRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                let resp = SandboxResponse::Error {
                    message: format!("invalid request: {}", e),
                };
                send_response(&mut writer, &resp).await?;
                continue;
            }
        };

        // Process request
        let response = match &request {
            SandboxRequest::Evaluate { request } => {
                SandboxResponse::PermissionResponse(engine.evaluate(request.clone()))
            }
            SandboxRequest::ReloadRules { rules: _ } => {
                // The engine is recreated externally; we just acknowledge.
                SandboxResponse::RulesReloaded
            }
            SandboxRequest::Ping => SandboxResponse::Pong,
        };

        send_response(&mut writer, &response).await?;
    }

    Ok(())
}

async fn send_response(
    writer: &mut OwnedWriteHalf,
    response: &SandboxResponse,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(response)?;
    let len = (json.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_request_ping_serialization() {
        let req = SandboxRequest::Ping;
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: SandboxRequest = serde_json::from_slice(&json).unwrap();
        // Compare JSON string representation since contained types may not implement PartialEq
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_response_pong_serialization() {
        let resp = SandboxResponse::Pong;
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: SandboxResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_response_error_serialization() {
        let resp = SandboxResponse::Error {
            message: "test error message".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: SandboxResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_request_evaluate_serialization() {
        let req = SandboxRequest::Evaluate {
            request: crate::permission::PermissionRequest::Bare(
                crate::permission::PermissionRequestBody::FileOp {
                    agent: "test-agent".to_string(),
                    path: "/tmp/test.txt".to_string(),
                    op: "read".to_string(),
                },
            ),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: SandboxRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_request_reload_rules_serialization() {
        let rules = crate::permission::engine::RuleSet {
            version: "1.0".to_string(),
            rules: vec![],
            defaults: Default::default(),
            template_includes: vec![],
            agent_creators: Default::default(),
        };
        let req = SandboxRequest::ReloadRules { rules };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: SandboxRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_response_permission_allowed_serialization() {
        let resp =
            SandboxResponse::PermissionResponse(crate::permission::PermissionResponse::Allowed {
                token: "token123".to_string(),
            });
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: SandboxResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_sandbox_response_rules_reloaded_serialization() {
        let resp = SandboxResponse::RulesReloaded;
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: SandboxResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_ipc_channel_new() {
        let channel = IpcChannel::new("/tmp/test-socket.sock");
        assert_eq!(
            channel.path,
            std::path::PathBuf::from("/tmp/test-socket.sock")
        );
    }

    #[test]
    fn test_ipc_channel_clean_up_nonexistent_path() {
        // clean_up on a non-existent path should not panic (idempotent)
        let channel = IpcChannel::new("/tmp/nonexistent-socket-12345.sock");
        channel.clean_up(); // should not panic
                            // Running it twice is also fine (idempotent)
        channel.clean_up();
    }
}
