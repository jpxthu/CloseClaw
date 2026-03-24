//! Per-connection chat session state

use crate::chat::protocol::{ClientMessage, ServerMessage};
use crate::llm::{ChatRequest, LLMRegistry, Message};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

/// Default LLM call timeout (60 seconds).
const DEFAULT_LLM_TIMEOUT_SECS: u64 = 60;

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
    /// LLM registry for chat completions
    llm_registry: Arc<LLMRegistry>,
    /// Default LLM provider name
    llm_provider: String,
    /// Default model name
    model: String,
    /// Chat history for context
    chat_history: Vec<Message>,
}

impl ChatSession {
    /// Create a new session for an accepted connection
    pub fn new(
        session_id: String,
        agent_id: String,
        stream: TcpStream,
        shutdown_rx: tokio::sync::broadcast::Receiver<()>,
        llm_registry: Arc<LLMRegistry>,
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
            llm_registry,
            llm_provider: std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "minimax".to_string()),
            model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "MiniMax-M2.5".to_string()),
            chat_history: Vec::new(),
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
                            let msgs = self.handle_line(line).await;
                            for msg in msgs {
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

    /// Handle a single incoming JSON line, return zero or more server messages to send back
    async fn handle_line(&mut self, line: &str) -> Vec<ServerMessage> {
        let msg: ClientMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => {
                warn!(session_id = %self.session_id, error = %e, raw = %line, "failed to parse client message");
                return vec![ServerMessage::ChatError {
                    message: format!("invalid message: {}", e),
                    id: uuid::Uuid::new_v4().to_string(),
                }];
            }
        };

        match msg {
            ClientMessage::ChatStart { agent_id, id } => {
                info!(session_id = %self.session_id, agent_id = %agent_id, id = %id, "session start requested");
                self.agent_id = agent_id;
                vec![ServerMessage::ChatStarted {
                    session_id: self.session_id.clone(),
                    id,
                }]
            }
            ClientMessage::ChatMessage { content, id } => {
                info!(session_id = %self.session_id, content_len = %content.len(), id = %id, "chat message received");

                // Add user message to history
                self.chat_history.push(Message {
                    role: "user".to_string(),
                    content: content.clone(),
                });

                // Call LLM
                let response = self.call_llm(&content).await;

                // Add assistant response to history
                if let Ok(ref resp) = response {
                    self.chat_history.push(Message {
                        role: "assistant".to_string(),
                        content: resp.clone(),
                    });
                }

                match response {
                    Ok(content) => vec![
                        ServerMessage::ChatResponse {
                            content,
                            done: true,
                            id: id.clone(),
                        },
                        ServerMessage::ChatResponseDone { id },
                    ],
                    Err(e) => {
                        error!(session_id = %self.session_id, error = %e, "LLM call failed");
                        vec![
                            ServerMessage::ChatResponse {
                                content: format!("[error] LLM call failed: {}", e),
                                done: true,
                                id: id.clone(),
                            },
                            ServerMessage::ChatResponseDone { id },
                        ]
                    }
                }
            }
            ClientMessage::ChatStop { id } => {
                info!(session_id = %self.session_id, id = %id, "session stop requested");
                self.active = false;
                vec![ServerMessage::ChatResponseDone { id }]
            }
        }
    }

    /// Call the LLM with the user's message and return the response content.
    /// Includes a timeout to prevent session blocking on slow/hanging API calls.
    async fn call_llm(&self, _content: &str) -> anyhow::Result<String> {
        // Get timeout from env var, default to 60s
        let timeout_secs: u64 = std::env::var("LLM_TIMEOUT_SECS")
            .unwrap_or_else(|_| DEFAULT_LLM_TIMEOUT_SECS.to_string())
            .parse()
            .unwrap_or(DEFAULT_LLM_TIMEOUT_SECS);
        let timeout = Duration::from_secs(timeout_secs);

        let provider = self.llm_registry.get(&self.llm_provider).await;

        let provider = match provider {
            Some(p) => p,
            None => {
                // Fallback: try any available provider
                let providers = self.llm_registry.list().await;
                if providers.is_empty() {
                    return Err(anyhow::anyhow!("no LLM providers configured"));
                }
                self.llm_registry
                    .get(&providers[0])
                    .await
                    .ok_or_else(|| anyhow::anyhow!("provider not found"))?
            }
        };

        let request = ChatRequest {
            model: self.model.clone(),
            messages: self.chat_history.clone(),
            temperature: 0.7,
            max_tokens: Some(2048),
        };

        // Wrap LLM call with timeout
        let response = tokio::time::timeout(
            timeout,
            provider.chat(request)
        )
        .await
        .map_err(|_| anyhow::anyhow!("LLM call timed out after {}s", timeout_secs))?
        .map_err(|e| anyhow::anyhow!("LLM error: {}", e))?;

        debug!(
            session_id = %self.session_id,
            model = %response.model,
            usage = ?response.usage,
            "LLM response received"
        );

        Ok(response.content)
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
