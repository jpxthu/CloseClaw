//! Per-connection chat session state

use crate::chat::protocol::{ClientMessage, ServerMessage};
use crate::llm::fallback::FallbackClient;
use crate::llm::{ChatRequest, LLMRegistry, Message};
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
    async fn handle_line(&mut self, line: &str) -> Vec<ServerMessage> {
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
                self.handle_chat_message(content, id).await
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

        self.chat_history.push(Message {
            role: "user".to_string(),
            content: content.clone(),
        });
        self.truncate_history();

        let response = self.call_llm().await;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chat::protocol::ServerMessage,
        llm::{LLMRegistry, StubProvider},
    };
    use std::sync::Arc;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        sync::broadcast,
    };

    /// Set up a ChatSession with a real TCP pair and StubProvider registered.
    pub(crate) async fn setup_session() -> (ChatSession, tokio::net::TcpStream) {
        std::env::set_var("LLM_FALLBACK_CHAIN", "stub/stub-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry
            .register("stub".to_string(), Arc::new(StubProvider::new()))
            .await;
        let session = ChatSession::new(
            "test-session".to_string(),
            "test-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        (session, client)
    }

    #[tokio::test]
    async fn test_truncate_history() {
        let (mut session, _client) = setup_session().await;
        session.max_history = 5;
        // Over limit: should remove oldest entries, keep newest 5 (5–9)
        session.chat_history = (0..10)
            .map(|i| crate::llm::Message {
                role: "user".to_string(),
                content: format!("msg {}", i),
            })
            .collect();
        session.truncate_history();
        assert_eq!(session.chat_history.len(), 5);
        assert_eq!(session.chat_history[0].content, "msg 5");
        assert_eq!(session.chat_history[4].content, "msg 9");
        // Under limit: no change
        session.chat_history = (0..3)
            .map(|i| crate::llm::Message {
                role: "user".to_string(),
                content: format!("msg {}", i),
            })
            .collect();
        session.truncate_history();
        assert_eq!(session.chat_history.len(), 3);
        // Exact limit: no change
        session.chat_history = (0..5)
            .map(|i| crate::llm::Message {
                role: "user".to_string(),
                content: format!("msg {}", i),
            })
            .collect();
        session.truncate_history();
        assert_eq!(session.chat_history.len(), 5);
        // Empty: no panic
        session.chat_history.clear();
        session.truncate_history();
        assert_eq!(session.chat_history.len(), 0);
    }

    #[tokio::test]
    async fn test_handle_line_chat_start() {
        let (mut session, _client) = setup_session().await;
        let json = r#"{"type":"chat.start","agent_id":"my-agent","id":"req1"}"#;
        let msgs = session.handle_line(json).await;
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            ServerMessage::ChatStarted { session_id, id } => {
                assert_eq!(session_id, "test-session");
                assert_eq!(id, "req1");
            }
            _ => panic!("expected ChatStarted"),
        }
        assert_eq!(session.agent_id, "my-agent");
    }

    #[tokio::test]
    async fn test_handle_line_chat_message() {
        let (mut session, _client) = setup_session().await;
        let json = r#"{"type":"chat.message","content":"hello","id":"msg1"}"#;
        let msgs = session.handle_line(json).await;
        assert_eq!(msgs.len(), 2);
        match &msgs[0] {
            ServerMessage::ChatResponse { content, done, id } => {
                assert!(done);
                assert_eq!(id, "msg1");
                assert_eq!(content, "stub response");
            }
            _ => panic!("expected ChatResponse"),
        }
        match &msgs[1] {
            ServerMessage::ChatResponseDone { id } => {
                assert_eq!(id, "msg1");
            }
            _ => panic!("expected ChatResponseDone"),
        }
        assert_eq!(session.chat_history.len(), 2);
        assert_eq!(session.chat_history[0].role, "user");
        assert_eq!(session.chat_history[0].content, "hello");
        assert_eq!(session.chat_history[1].role, "assistant");
    }
    #[tokio::test]
    async fn test_handle_line_chat_stop() {
        let (mut session, _client) = setup_session().await;
        assert!(session.active);
        let json = r#"{"type":"chat.stop","id":"stop1"}"#;
        let msgs = session.handle_line(json).await;
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            ServerMessage::ChatResponseDone { id } => {
                assert_eq!(id, "stop1");
            }
            _ => panic!("expected ChatResponseDone"),
        }
        assert!(!session.active, "session should be inactive after stop");
    }

    #[tokio::test]
    async fn test_handle_line_invalid_json() {
        let (mut session, _client) = setup_session().await;
        let msgs = session.handle_line("not valid json at all").await;
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            ServerMessage::ChatError { message, .. } => {
                assert!(message.contains("invalid message"));
            }
            _ => panic!("expected ChatError"),
        }
    }

    #[tokio::test]
    async fn test_send_message_writes_json() {
        let (mut session, mut client) = setup_session().await;
        let msg = ServerMessage::ChatResponse {
            content: "hello world".to_string(),
            done: true,
            id: "req-x".to_string(),
        };
        session.send_message(msg).await.unwrap();
        drop(session);
        let mut reader = tokio::io::BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.contains(r#""type":"chat.response"#));
        assert!(line.contains("hello world"));
        assert!(line.ends_with("\n"));
    }

    #[tokio::test]
    async fn test_chat_session_new_fields() {
        std::env::set_var("LLM_FALLBACK_CHAIN", "stub/stub-model");
        std::env::set_var("LLM_MODEL", "custom-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        registry
            .register("stub".to_string(), Arc::new(StubProvider::new()))
            .await;
        let session = ChatSession::new(
            "sid-123".to_string(),
            "aid-456".to_string(),
            stream,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        std::env::remove_var("LLM_MODEL");
        assert_eq!(session.session_id, "sid-123");
        assert_eq!(session.agent_id, "aid-456");
        assert_eq!(session.model, "custom-model");
        assert!(session.active);
        assert_eq!(session.max_history, 100);
    }

    #[cfg(feature = "fake-llm")]
    #[tokio::test]
    async fn test_handle_chat_message_llm_failure() {
        use crate::llm::fake::FakeProvider;
        use crate::llm::LLMError;

        std::env::set_var("LLM_FALLBACK_CHAIN", "fake/fake-model");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        let registry = Arc::new(LLMRegistry::new());
        let fake = FakeProvider::builder()
            .then_err(LLMError::InvalidRequest("test error".to_string()))
            .build();
        registry.register("fake".to_string(), Arc::new(fake)).await;
        let mut session = ChatSession::new(
            "fail-session".to_string(),
            "fail-agent".to_string(),
            accepted,
            shutdown_rx.resubscribe(),
            registry,
        );
        std::env::remove_var("LLM_FALLBACK_CHAIN");

        let json = r#"{"type":"chat.message","content":"hello","id":"fail1"}"#;
        let msgs = session.handle_line(json).await;
        assert_eq!(msgs.len(), 2);
        match &msgs[0] {
            ServerMessage::ChatResponse { content, done, .. } => {
                assert!(done);
                assert!(
                    content.contains("[error]"),
                    "expected [error], got: {}",
                    content
                );
            }
            _ => panic!("expected ChatResponse with error"),
        }
    }
}
