//! Per-connection chat session state

use crate::chat::protocol::{ClientMessage, ServerMessage};
use crate::llm::fallback::FallbackClient;
use crate::llm::{ChatRequest, LLMRegistry, Message};
use crate::session::compaction::{execute_compact, CompactConfig, CompactionService};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

/// Default max chat history entries (100 messages = ~50 turns).
const DEFAULT_MAX_HISTORY: usize = 100;

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
    /// LLM fallback client (handles retry + fallback chain)
    fallback_client: Arc<FallbackClient>,
    /// Default model name (used for the ChatRequest)
    model: String,
    /// Chat history for context
    pub chat_history: Vec<Message>,
    /// Max chat history entries (to prevent unbounded memory growth).
    pub max_history: usize,
    /// Session compaction service with auto-trigger and circuit breaker.
    pub(crate) compaction_service: CompactionService,
}

// --- Construction ---

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

        let max_history: usize = std::env::var("CHAT_MAX_HISTORY")
            .unwrap_or_else(|_| DEFAULT_MAX_HISTORY.to_string())
            .parse()
            .unwrap_or(DEFAULT_MAX_HISTORY);

        let fallback_chain: Vec<String> = std::env::var("LLM_FALLBACK_CHAIN")
            .map(|s| s.split(',').map(str::trim).map(String::from).collect())
            .unwrap_or_else(|_| {
                let provider =
                    std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "minimax".to_string());
                let model =
                    std::env::var("LLM_MODEL").unwrap_or_else(|_| "MiniMax-M2.5".to_string());
                vec![format!("{}/{}", provider, model)]
            });

        let timeout_secs: u64 = std::env::var("LLM_TIMEOUT_SECS")
            .unwrap_or_else(|_| "30".to_string())
            .parse()
            .unwrap_or(30);

        let fallback_client = Arc::new(
            FallbackClient::from_strings(Arc::clone(&llm_registry), fallback_chain)
                .with_timeout(timeout_secs),
        );

        Self {
            session_id,
            agent_id,
            reader,
            writer,
            active: true,
            shutdown_rx,
            fallback_client,
            model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "MiniMax-M2.5".to_string()),
            chat_history: Vec::new(),
            max_history,
            compaction_service: CompactionService::new(CompactConfig::default()),
        }
    }

    /// Truncate chat history to max_history entries, keeping the most recent messages.
    pub fn truncate_history(&mut self) {
        if self.chat_history.len() > self.max_history {
            let remove_count = self.chat_history.len() - self.max_history;
            self.chat_history.drain(0..remove_count);
            debug!(
                session_id = %self.session_id,
                removed = %remove_count,
                remaining = %self.chat_history.len(),
                "chat history truncated"
            );
        }
    }
}

// --- Main loop ---

impl ChatSession {
    /// Run the session loop — read client messages, dispatch to agent, stream responses
    pub async fn run(mut self) {
        info!(session_id = %self.session_id, agent_id = %self.agent_id, "chat session started");

        let mut shutdown_rx = self.shutdown_rx.resubscribe();

        loop {
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
                            if line.is_empty() { continue; }
                            debug!(session_id = %self.session_id, line = %line, "received line");
                            let msgs = self.handle_line(line).await;
                            for msg in msgs {
                                if let Err(e) = self.send_message(msg).await {
                                    error!(session_id = %self.session_id,
                                           error = %e, "send failed");
                                    break;
                                }
                            }
                            line_buf.clear();
                        }
                        Err(e) => {
                            error!(session_id = %self.session_id, error = %e, "read error");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!(session_id = %self.session_id, "session shutting down");
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

    /// Send a ServerMessage to the client, appending a newline
    async fn send_message(&mut self, msg: ServerMessage) -> anyhow::Result<()> {
        let json = msg.to_json()?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}

// --- Message handling ---

impl ChatSession {
    /// Handle a single incoming JSON line, return zero or more server messages to send back
    pub(crate) async fn handle_line(&mut self, line: &str) -> Vec<ServerMessage> {
        let msg: ClientMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => {
                warn!(session_id = %self.session_id, error = %e, raw = %line, "parse error");
                return vec![ServerMessage::ChatError {
                    message: format!("invalid message: {}", e),
                    id: uuid::Uuid::new_v4().to_string(),
                }];
            }
        };

        match msg {
            ClientMessage::ChatStart { agent_id, id } => {
                info!(session_id = %self.session_id,
                      agent_id = %agent_id, id = %id, "session start");
                self.agent_id = agent_id;
                vec![ServerMessage::ChatStarted {
                    session_id: self.session_id.clone(),
                    id,
                }]
            }
            ClientMessage::ChatMessage { content, id } => {
                if content.trim_start().starts_with("/compact") {
                    self.handle_compact_command(content, id).await
                } else {
                    self.handle_chat_message(content, id).await
                }
            }
            ClientMessage::ChatStop { id } => {
                info!(session_id = %self.session_id, id = %id, "session stop");
                self.active = false;
                vec![ServerMessage::ChatResponseDone { id }]
            }
        }
    }

    /// Process a ChatMessage: update history, call LLM, return response messages
    async fn handle_chat_message(&mut self, content: String, id: String) -> Vec<ServerMessage> {
        info!(session_id = %self.session_id,
               content_len = %content.len(), id = %id, "chat message");

        self.push_history(content.clone());
        self.truncate_history();

        // Auto-compaction check: after truncate, before calling LLM
        if self.should_auto_compact_and_execute().await {
            // history replaced with summary
        }

        // Call LLM and return response
        let response = self.call_llm().await;
        if let Ok(ref resp) = response {
            self.chat_history.push(Message {
                role: "assistant".to_string(),
                content: resp.clone(),
            });
        }

        self.response_to_messages(response, id)
    }

    /// Push a user message to history
    fn push_history(&mut self, content: String) {
        self.chat_history.push(Message {
            role: "user".to_string(),
            content,
        });
    }

    /// Check and execute auto-compaction; returns true if compaction ran and replaced history
    async fn should_auto_compact_and_execute(&mut self) -> bool {
        if !self
            .compaction_service
            .should_auto_compact(&self.chat_history, &self.model)
        {
            return false;
        }

        match execute_compact(
            &self.chat_history,
            &*self.fallback_client,
            &self.model,
            None,
            true,
        )
        .await
        {
            Ok(comp_result) => {
                self.chat_history = vec![Message {
                    role: "system".to_string(),
                    content: comp_result.boundary_message,
                }];
                self.compaction_service.record_success();
                debug!(session_id = %self.session_id, before_tokens = %comp_result.before_token_count, after_tokens = %comp_result.after_token_count, "auto compaction succeeded");
                true
            }
            Err(e) => {
                self.compaction_service.record_failure();
                warn!(session_id = %self.session_id, error = %e, "auto compaction failed, continuing normally");
                false
            }
        }
    }

    /// Convert LLM response to ServerMessages
    fn response_to_messages(
        &self,
        response: Result<String, anyhow::Error>,
        id: String,
    ) -> Vec<ServerMessage> {
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

    /// Call the LLM with the current chat history and return the response content
    async fn call_llm(&self) -> anyhow::Result<String> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: self.chat_history.clone(),
            temperature: 0.7,
            max_tokens: Some(2048),
        };

        let response = self
            .fallback_client
            .chat(request)
            .await
            .map_err(|e| anyhow::anyhow!("LLM error: {}", e))?;

        debug!(session_id = %self.session_id, model = %response.model,
               usage = ?response.usage, "LLM response");
        Ok(response.content)
    }

    /// Handle a /compact command: run compaction and return results.
    async fn handle_compact_command(&mut self, content: String, id: String) -> Vec<ServerMessage> {
        let cmd = match crate::mode::slash_command::parse_slash_command(&content) {
            Some(c) => c,
            None => return self.invalid_compact_cmd(&id),
        };
        let _ = crate::mode::slash_command::handle_slash_command(&cmd);
        // Execute compaction
        match execute_compact(
            &self.chat_history,
            &*self.fallback_client,
            &self.model,
            None,
            false,
        )
        .await
        {
            Ok(result) => {
                self.set_history_to_compact_result(&result);
                self.compaction_service.record_success();
                self.compact_success_msg(&result, &id)
            }
            Err(e) => {
                warn!(session_id = %self.session_id, error = %e, "/compact failed");
                self.compaction_service.record_failure();
                self.compact_error_msg(&e, &id)
            }
        }
    }

    /// Return invalid command error
    fn invalid_compact_cmd(&self, id: &str) -> Vec<ServerMessage> {
        vec![
            ServerMessage::ChatResponse {
                content: "[error] invalid /compact command".to_string(),
                done: true,
                id: id.to_string(),
            },
            ServerMessage::ChatResponseDone { id: id.to_string() },
        ]
    }

    /// Replace history with compaction result
    fn set_history_to_compact_result(
        &mut self,
        result: &crate::session::compaction::CompactionResult,
    ) {
        self.chat_history = vec![Message {
            role: "system".to_string(),
            content: result.boundary_message.clone(),
        }];
    }

    /// Build success message for compaction
    fn compact_success_msg(
        &self,
        result: &crate::session::compaction::CompactionResult,
        id: &str,
    ) -> Vec<ServerMessage> {
        vec![
            ServerMessage::ChatResponse {
                content: format!(
                    "✅ 压缩成功: 共 {} 条消息 ({} tokens → {} tokens)",
                    self.chat_history.len(),
                    result.before_token_count,
                    result.after_token_count
                ),
                done: true,
                id: id.to_string(),
            },
            ServerMessage::ChatResponseDone { id: id.to_string() },
        ]
    }

    /// Build error message for compaction
    fn compact_error_msg(
        &self,
        e: &crate::session::compaction::CompactionError,
        id: &str,
    ) -> Vec<ServerMessage> {
        vec![
            ServerMessage::ChatResponse {
                content: format!("[error] 压缩失败: {}", e),
                done: true,
                id: id.to_string(),
            },
            ServerMessage::ChatResponseDone { id: id.to_string() },
        ]
    }
}
