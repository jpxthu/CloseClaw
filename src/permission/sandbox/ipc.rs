//! IPC channel for sandbox engine communication.

use std::path::PathBuf;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

use super::{SandboxRequest, SandboxResponse, IPC_TIMEOUT_MS};

/// IPC channel for communicating with the sandbox engine.
pub struct IpcChannel {
    pub(crate) path: PathBuf,
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
    pub async fn serve(
        self,
        engine: std::sync::Arc<super::PermissionEngine>,
    ) -> std::io::Result<()> {
        self.clean_up();

        let listener = UnixListener::bind(&self.path)?;

        tracing::info!("engine IPC server listening on {}", self.path.display());

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let engine = engine.clone();
                    tokio::spawn(super::handle_connection(stream, engine));
                }
                Err(e) => {
                    tracing::error!("UnixListener accept error: {}", e);
                }
            }
        }
    }
}
