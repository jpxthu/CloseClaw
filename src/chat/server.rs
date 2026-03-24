//! Chat TCP server — accepts connections and dispatches to sessions

use crate::chat::session::ChatSession;
use crate::llm::LLMRegistry;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::{error, info};

/// Chat server bind address
const CHAT_BIND_ADDR: &str = "127.0.0.1:18889";
/// Default agent for chat sessions (when client doesn't specify).
const DEFAULT_AGENT_ID: &str = "guide";

/// ChatServer — TCP server that handles incoming chat connections
pub struct ChatServer {
    /// Shutdown broadcast channel
    shutdown_tx: broadcast::Sender<()>,
    /// LLM registry for chat completions
    llm_registry: Arc<LLMRegistry>,
}

impl ChatServer {
    /// Create a new ChatServer with the given LLM registry
    pub fn new(llm_registry: Arc<LLMRegistry>) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            shutdown_tx,
            llm_registry,
        }
    }

    /// Run the server — accepts connections until `shutdown_rx` is triggered
    pub async fn run(&self, mut shutdown_rx: broadcast::Receiver<()>) -> anyhow::Result<()> {
        let listener = TcpListener::bind(CHAT_BIND_ADDR).await?;
        info!(addr = %CHAT_BIND_ADDR, "chat TCP server listening");

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            let session_id = uuid::Uuid::new_v4().to_string();
                            let shutdown_rx = self.shutdown_tx.subscribe();
                            let llm_registry = Arc::clone(&self.llm_registry);
                            info!(session_id = %session_id, client = %addr, "new chat connection");

                            tokio::spawn(async move {
                                let session = ChatSession::new(
                                    session_id,
                                    DEFAULT_AGENT_ID.to_string(),
                                    stream,
                                    shutdown_rx,
                                    llm_registry,
                                );
                                session.run().await;
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "error accepting TCP connection");
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("chat server received shutdown signal");
                    break;
                }
            }
        }

        info!("chat TCP server stopped");
        Ok(())
    }

    /// Signal the server to stop
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

impl Default for ChatServer {
    fn default() -> Self {
        Self::new(Arc::new(LLMRegistry::new()))
    }
}

/// Spawn the chat server as a background task, returning a handle to it
pub fn spawn_chat_server(llm_registry: Arc<LLMRegistry>) -> ChatServer {
    ChatServer::new(llm_registry)
}
