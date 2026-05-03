//! Chat TCP server — accepts connections and dispatches to sessions

use crate::chat::session::ChatSession;
use crate::llm::LLMRegistry;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::{error, info};

/// Default chat server bind address
const DEFAULT_CHAT_BIND_ADDR: &str = "127.0.0.1:18889";
/// Default agent for chat sessions (when client doesn't specify).
const DEFAULT_AGENT_ID: &str = "guide";

/// ChatServer — TCP server that handles incoming chat connections
pub struct ChatServer {
    /// Shutdown broadcast channel
    shutdown_tx: broadcast::Sender<()>,
    /// LLM registry for chat completions
    llm_registry: Arc<LLMRegistry>,
    /// Bind address for the TCP listener
    bind_addr: String,
}

impl ChatServer {
    /// Create a new ChatServer with the given LLM registry and optional bind address.
    /// If `bind_addr` is `None`, defaults to `127.0.0.1:18889`.
    pub fn new(llm_registry: Arc<LLMRegistry>, bind_addr: Option<&str>) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        let bind_addr = bind_addr.unwrap_or(DEFAULT_CHAT_BIND_ADDR).to_string();
        Self {
            shutdown_tx,
            llm_registry,
            bind_addr,
        }
    }

    /// Run the server — accepts connections until `shutdown_rx` is triggered
    pub async fn run(&self, mut shutdown_rx: broadcast::Receiver<()>) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        info!(addr = %self.bind_addr, "chat TCP server listening");

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
        Self::new(Arc::new(LLMRegistry::new()), None)
    }
}

/// Spawn the chat server as a background task, returning a handle to it
pub fn spawn_chat_server(llm_registry: Arc<LLMRegistry>) -> ChatServer {
    ChatServer::new(llm_registry, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_server_new() {
        let registry = Arc::new(LLMRegistry::new());
        let server = ChatServer::new(registry, None);
        // shutdown should work without panic
        server.shutdown();
    }

    #[test]
    fn test_chat_server_with_custom_addr() {
        let registry = Arc::new(LLMRegistry::new());
        let server = ChatServer::new(registry, Some("127.0.0.1:0"));
        // bind_addr should be set to the custom value
        assert_eq!(server.bind_addr, "127.0.0.1:0");
        server.shutdown();
    }

    #[test]
    fn test_chat_server_new_with_default_addr() {
        let registry = Arc::new(LLMRegistry::new());
        let server = ChatServer::new(registry, None);
        assert_eq!(server.bind_addr, DEFAULT_CHAT_BIND_ADDR);
        server.shutdown();
    }

    #[test]
    fn test_chat_server_default() {
        let server = ChatServer::default();
        assert_eq!(server.bind_addr, DEFAULT_CHAT_BIND_ADDR);
        server.shutdown();
    }

    #[test]
    fn test_spawn_chat_server() {
        let registry = Arc::new(LLMRegistry::new());
        let server = spawn_chat_server(registry);
        server.shutdown();
    }

    #[test]
    fn test_shutdown_multiple_times() {
        let server = ChatServer::default();
        server.shutdown();
        // second shutdown should not panic
        server.shutdown();
    }
}
