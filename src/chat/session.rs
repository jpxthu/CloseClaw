//! Per-connection chat session state

use crate::chat::protocol::{ClientMessage, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

/// Chat session — handles messages for a single TCP connection
pub struct ChatSession {
    /// Unique session ID assigned by the server
    pub session_id: String,
    /// The agent_id requested by the client
    pub agent_id: String,
    /// Buffered line reader from the client
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    /// Write half to the client
    writer: tokio::net::tcp::OwnedWriteHalf,
    /// Flag indicating the session is active
    active: bool,
    /// Shutdown signal receiver
    shutdown_rx: tokio::sync::broadcast::Receiver<()>,
}

impl ChatSession {
    /// Create a new session for an accepted connection
    pub fn new(
        session_id: String,
        agent_id: String,
        stream: TcpStream,
        shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) -> Self {
        let (reader, writer) = stream.into_split();
        let reader = BufReader::new(reader);

        Self {
            session_id,
            agent_id,
            reader,
            writer,
            active: true,
            shutdown_rx,
        }
    }

    /// Run the session loop — read client messages, dispatch to agent, stream responses
    pub async fn run(mut self) {
        info!(
            session_id = %self.session_id,
            agent_id = %self.agent_id,
            "chat session started"
        );

        let mut shutdown_rx = self.shutdown_rx.resubscribe();

        loop {
            // Use read_line on the BufReader directly so we keep &mut self
            let mut line_buf = String::new();

            tokio::select! {
                read_result = self.reader.read_line(&mut line_buf) => {
                    match read_result {
                        Ok(0) => {
                            info!(session_id = %self.session_id, "client disconnected");
                            break;
                        }
                        Ok(_) => {
                            let line = line_buf.trim_end();
                            if line.is_empty() {
                                continue;
                            }
                            debug!(session_id = %self.session_id, line = %line, "received line");
                            let response = self.handle_line(line).await;
                            if let Some(msg) = response {
                                if let Err(e) = self.send_message(msg).await {
                                    error!(session_id = %self.session_id, error = %e, "failed to send to client");
                                    break;
                                }
                            }
                            line_buf.clear();
                        }
                        Err(e) => {
                            error!(session_id = %self.session_id, error = %e, "error reading from client");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!(session_id = %self.session_id, "session shutting down due to server shutdown");
                    let _ = self.send_message(ServerMessage::ChatError {
                        message: "server shutting down".to_string(),
                        id: uuid::Uuid::new_v4().to_string(),
                    }).await;
                    break;
                }
            }
        }

        info!(session_id = %self.session_id, "chat session ended");
    }

    /// Handle a single incoming JSON line, return optional server message to send back
    async fn handle_line(&mut self, line: &str) -> Option<ServerMessage> {
        let msg: ClientMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => {
                warn!(session_id = %self.session_id, error = %e, raw = %line, "failed to parse client message");
                return Some(ServerMessage::ChatError {
                    message: format!("invalid message: {}", e),
                    id: uuid::Uuid::new_v4().to_string(),
                });
            }
        };

        match msg {
            ClientMessage::ChatStart { agent_id, id } => {
                info!(session_id = %self.session_id, agent_id = %agent_id, id = %id, "session start requested");
                self.agent_id = agent_id;
                Some(ServerMessage::ChatStarted {
                    session_id: self.session_id.clone(),
                    id,
                })
            }
            ClientMessage::ChatMessage { content, id } => {
                info!(session_id = %self.session_id, content_len = %content.len(), id = %id, "chat message received");
                Some(ServerMessage::ChatResponse {
                    content: format!("[echo] {}", content),
                    done: false,
                    id,
                })
            }
            ClientMessage::ChatStop { id } => {
                info!(session_id = %self.session_id, id = %id, "session stop requested");
                self.active = false;
                Some(ServerMessage::ChatResponseDone { id })
            }
        }
    }

    /// Send a ServerMessage to the client, appending a newline
    async fn send_message(&mut self, msg: ServerMessage) -> anyhow::Result<()> {
        let json = msg.to_json()?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}
