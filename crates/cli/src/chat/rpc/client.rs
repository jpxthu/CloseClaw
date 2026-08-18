//! Chat RPC client — connects to the daemon's chat socket and sends
//! requests, receiving streaming responses.
//!
//! Uses length-prefixed JSON frames over a Unix domain socket:
//! ```text
//! [4-byte big-endian length (u32)][JSON frame bytes]
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

use super::protocol::{ChatRequest, ChatResponse};

/// Default timeout for chat RPC operations (milliseconds).
const CHAT_TIMEOUT_MS: u64 = 10_000;

/// Chat RPC client that connects to the daemon's chat socket.
pub struct ChatRpcClient {
    socket_path: PathBuf,
    timeout_ms: u64,
}

impl ChatRpcClient {
    /// Create a new client targeting the given socket path.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout_ms: CHAT_TIMEOUT_MS,
        }
    }

    /// Create a client with a custom timeout.
    pub fn with_timeout(socket_path: impl Into<PathBuf>, timeout_ms: u64) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout_ms,
        }
    }

    /// Send a user message and return a streaming response reader.
    ///
    /// The returned [`ChatResponseStream`] yields [`ChatResponse`] items
    /// one at a time until a [`ChatResponse::Done`] is received.
    pub async fn send_message(
        &self,
        agent_id: &str,
        content: &str,
    ) -> std::io::Result<ChatResponseStream> {
        let stream = self.connect_inner().await?;
        let (reader, mut writer) = stream.into_split();

        let request = ChatRequest::ChatMessage {
            agent_id: agent_id.to_string(),
            content: content.to_string(),
        };
        send_frame(&mut writer, &request).await?;

        Ok(ChatResponseStream {
            reader: BufReader::new(reader),
            writer,
        })
    }

    /// Send a stop-session request and return the server's response.
    pub async fn stop_session(&self, agent_id: &str) -> std::io::Result<ChatResponse> {
        let stream = self.connect_inner().await?;
        let (reader, mut writer) = stream.into_split();

        let request = ChatRequest::StopSession {
            agent_id: agent_id.to_string(),
        };
        send_frame(&mut writer, &request).await?;

        let mut reader = BufReader::new(reader);
        read_frame(&mut reader).await
    }

    /// Send a quit request and return the server's response.
    pub async fn quit(&self) -> std::io::Result<ChatResponse> {
        let stream = self.connect_inner().await?;
        let (reader, mut writer) = stream.into_split();

        send_frame(&mut writer, &ChatRequest::Quit).await?;

        let mut reader = BufReader::new(reader);
        read_frame(&mut reader).await
    }

    /// Attempt a lightweight connect to verify the daemon is reachable.
    pub async fn ping(&self) -> bool {
        self.quit().await.is_ok()
    }

    /// Connect to the chat socket with the configured timeout.
    async fn connect_inner(&self) -> std::io::Result<UnixStream> {
        timeout(
            Duration::from_millis(self.timeout_ms),
            UnixStream::connect(&self.socket_path),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "chat RPC connect timeout")
        })?
    }
}

/// Streaming response reader for a single chat message exchange.
///
/// Yields [`ChatResponse`] frames until [`ChatResponse::Done`] is received.
pub struct ChatResponseStream {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl ChatResponseStream {
    /// Read the next response from the daemon.
    ///
    /// Returns `Ok(None)` when the stream has ended (connection closed or
    /// `Done` received).
    pub async fn next(&mut self) -> std::io::Result<Option<ChatResponse>> {
        match read_frame(&mut self.reader).await {
            Ok(ChatResponse::Done) => Ok(None),
            Ok(resp) => Ok(Some(resp)),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Send a follow-up request on the same connection.
    pub async fn send(&mut self, request: &ChatRequest) -> std::io::Result<()> {
        send_frame(&mut self.writer, request).await
    }
}

/// Resolve the chat socket path from the config directory.
pub fn chat_socket_path(config_dir: &Path) -> PathBuf {
    config_dir.join("chat.sock")
}

// ── Frame I/O helpers ───────────────────────────────────────────────

/// Write a length-prefixed JSON frame.
async fn send_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    frame: &ChatRequest,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(frame)?;
    let len = (json.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a length-prefixed JSON frame.
async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> std::io::Result<ChatResponse> {
    let mut hdr = [0u8; 4];
    reader.read_exact(&mut hdr).await?;
    let body_len = u32::from_be_bytes(hdr) as usize;

    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).await?;

    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::net::UnixListener;

    #[test]
    fn test_client_new() {
        let client = ChatRpcClient::new("/tmp/test.sock");
        assert_eq!(client.socket_path, PathBuf::from("/tmp/test.sock"));
        assert_eq!(client.timeout_ms, CHAT_TIMEOUT_MS);
    }

    #[test]
    fn test_client_with_timeout() {
        let client = ChatRpcClient::with_timeout("/tmp/test.sock", 2000);
        assert_eq!(client.socket_path, PathBuf::from("/tmp/test.sock"));
        assert_eq!(client.timeout_ms, 2000);
    }

    #[test]
    fn test_chat_socket_path() {
        let path = chat_socket_path(Path::new("/home/user/.closeclaw"));
        assert_eq!(path, PathBuf::from("/home/user/.closeclaw/chat.sock"));
    }

    #[tokio::test]
    async fn test_client_connect_to_nonexistent_socket() {
        let client = ChatRpcClient::with_timeout("/tmp/nonexistent-chat-test.sock", 100);
        let result = client.quit().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_client_ping_to_nonexistent_socket() {
        let client = ChatRpcClient::with_timeout("/tmp/nonexistent-chat-test.sock", 100);
        assert!(!client.ping().await);
    }

    /// Helper: spin up a mock chat RPC server and return the socket path.
    ///
    /// The server calls `handler` for each received request and sends back
    /// the returned response. `shutdown` is triggered when the handler
    /// returns `None`.
    async fn spawn_mock_server(
        handler: Arc<dyn Fn(ChatRequest) -> Option<ChatResponse> + Send + Sync + 'static>,
    ) -> PathBuf {
        let sock = tempfile::tempdir().unwrap().keep().join("test-chat.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handler = Arc::clone(&handler);

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut reader = BufReader::new(reader);

                    loop {
                        let mut hdr = [0u8; 4];
                        match reader.read_exact(&mut hdr).await {
                            Ok(_) => {}
                            Err(_) => break,
                        }
                        let body_len = u32::from_be_bytes(hdr) as usize;
                        let mut body = vec![0u8; body_len];
                        if reader.read_exact(&mut body).await.is_err() {
                            break;
                        }

                        let request: ChatRequest = match serde_json::from_slice(&body) {
                            Ok(r) => r,
                            Err(_) => break,
                        };

                        match handler(request) {
                            Some(response) => {
                                let json = serde_json::to_vec(&response).unwrap();
                                let len = (json.len() as u32).to_be_bytes();
                                let _ = writer.write_all(&len).await;
                                let _ = writer.write_all(&json).await;
                                let _ = writer.flush().await;
                            }
                            None => break,
                        }
                    }
                });
            }
        });

        // Give the listener a moment to bind.
        tokio::time::sleep(Duration::from_millis(10)).await;
        sock
    }

    #[tokio::test]
    async fn test_quit_roundtrip() {
        let handler: Arc<dyn Fn(ChatRequest) -> Option<ChatResponse> + Send + Sync> =
            Arc::new(|req| match req {
                ChatRequest::Quit => Some(ChatResponse::Done),
                _ => Some(ChatResponse::Error {
                    message: "unexpected".to_string(),
                }),
            });
        let sock = spawn_mock_server(handler).await;
        let client = ChatRpcClient::with_timeout(&sock, 5000);
        let resp = client.quit().await.unwrap();
        assert!(matches!(resp, ChatResponse::Done));
    }

    #[tokio::test]
    async fn test_stop_session_roundtrip() {
        let handler: Arc<dyn Fn(ChatRequest) -> Option<ChatResponse> + Send + Sync> =
            Arc::new(|req| match req {
                ChatRequest::StopSession { .. } => Some(ChatResponse::Done),
                _ => Some(ChatResponse::Error {
                    message: "unexpected".to_string(),
                }),
            });
        let sock = spawn_mock_server(handler).await;
        let client = ChatRpcClient::with_timeout(&sock, 5000);
        let resp = client.stop_session("agent-1").await.unwrap();
        assert!(matches!(resp, ChatResponse::Done));
    }

    /// Helper: spin up a mock server that sends multiple frames per request.
    ///
    /// `handler` receives a request and returns `(Vec<ChatResponse>, bool)`
    /// where the bool indicates whether the connection should be closed
    /// after sending all responses (true = close, false = keep reading).
    async fn spawn_streaming_mock_server(
        handler: Arc<dyn Fn(ChatRequest) -> (Vec<ChatResponse>, bool) + Send + Sync + 'static>,
    ) -> PathBuf {
        let sock = tempfile::tempdir().unwrap().keep().join("test-chat.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handler = Arc::clone(&handler);

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut reader = BufReader::new(reader);

                    loop {
                        // Read request frame.
                        let mut hdr = [0u8; 4];
                        match reader.read_exact(&mut hdr).await {
                            Ok(_) => {}
                            Err(_) => break,
                        }
                        let body_len = u32::from_be_bytes(hdr) as usize;
                        let mut body = vec![0u8; body_len];
                        if reader.read_exact(&mut body).await.is_err() {
                            break;
                        }

                        let request: ChatRequest = match serde_json::from_slice(&body) {
                            Ok(r) => r,
                            Err(_) => break,
                        };

                        let (responses, done) = handler(request);
                        for resp in responses {
                            let json = serde_json::to_vec(&resp).unwrap();
                            let len = (json.len() as u32).to_be_bytes();
                            if writer.write_all(&len).await.is_err() {
                                return;
                            }
                            if writer.write_all(&json).await.is_err() {
                                return;
                            }
                            if writer.flush().await.is_err() {
                                return;
                            }
                        }
                        if done {
                            break;
                        }
                    }
                    // Dropping writer closes the socket, signaling EOF to the client.
                });
            }
        });

        // Give the listener a moment to bind.
        tokio::time::sleep(Duration::from_millis(10)).await;
        sock
    }

    #[tokio::test]
    async fn test_send_message_streaming() {
        let handler: Arc<dyn Fn(ChatRequest) -> (Vec<ChatResponse>, bool) + Send + Sync> =
            Arc::new(|req| match req {
                ChatRequest::ChatMessage { content, .. } => (
                    vec![ChatResponse::ContentChunk {
                        text: format!("echo: {}", content),
                    }],
                    true,
                ),
                _ => (vec![ChatResponse::Done], true),
            });
        let sock = spawn_streaming_mock_server(handler).await;
        let client = ChatRpcClient::with_timeout(&sock, 5000);

        let mut stream = client.send_message("test-agent", "hi").await.unwrap();
        let resp = stream.next().await.unwrap().unwrap();
        match resp {
            ChatResponse::ContentChunk { text } => {
                assert_eq!(text, "echo: hi");
            }
            other => panic!("expected ContentChunk, got {:?}", other),
        }
        // Server closed connection; next read should return None (EOF).
        let done = stream.next().await.unwrap();
        assert!(done.is_none());
    }

    #[tokio::test]
    async fn test_send_message_multiple_chunks() {
        let handler: Arc<dyn Fn(ChatRequest) -> (Vec<ChatResponse>, bool) + Send + Sync> =
            Arc::new(|req| match req {
                ChatRequest::ChatMessage { .. } => (
                    vec![
                        ChatResponse::ContentChunk {
                            text: "chunk-1".to_string(),
                        },
                        ChatResponse::ContentChunk {
                            text: "chunk-2".to_string(),
                        },
                        ChatResponse::Done,
                    ],
                    true,
                ),
                _ => (vec![ChatResponse::Done], true),
            });
        let sock = spawn_streaming_mock_server(handler).await;
        let client = ChatRpcClient::with_timeout(&sock, 5000);

        let mut stream = client.send_message("a", "b").await.unwrap();

        let r1 = stream.next().await.unwrap().unwrap();
        assert_eq!(
            r1,
            ChatResponse::ContentChunk {
                text: "chunk-1".to_string(),
            }
        );

        let r2 = stream.next().await.unwrap().unwrap();
        assert_eq!(
            r2,
            ChatResponse::ContentChunk {
                text: "chunk-2".to_string(),
            }
        );

        let done = stream.next().await.unwrap();
        assert!(done.is_none());
    }
}
