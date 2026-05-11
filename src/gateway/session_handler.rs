//! SessionMessageHandler - Gateway-layer LLM session manager with busy/pending state.
//!
//! This component implements the complete busy/pending messaging loop:
//! - idle message  → set busy → LLM call → clear busy → drain pending
//! - busy message  → enqueue pending
//!
//! `FallbackClient::chat()` (non-streaming) is used for all LLM calls.
//! The `output_tx` channel is used to surface LLM response text to callers.

use crate::gateway::session_manager::SessionManager;
use crate::llm::fallback::FallbackClient;
use crate::llm::{ChatRequest, ChatResponse, Message as ChatMessage};
use crate::session::persistence::PendingMessage;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Outcome of handling an inbound message.
#[derive(Debug)]
pub enum HandleResult {
    /// Message was queued behind an in-progress LLM call.
    MessageQueued,
    /// LLM call was started; response text will be sent to `output_tx`.
    LlmStarted,
}

/// Gateway-layer LLM session manager.
///
/// Manages `llm_busy` and `pending_messages` per session. Delegates LLM calls
/// to `FallbackClient` and surfaces responses via `output_tx`.
pub struct SessionMessageHandler {
    session_manager: Arc<SessionManager>,
    fallback_client: Arc<FallbackClient>,
    /// Channel for LLM response text. `None` means responses are discarded.
    output_tx: Arc<RwLock<Option<mpsc::Sender<String>>>>,
}

impl SessionMessageHandler {
    /// Create a new handler with an output channel.
    pub fn new(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<FallbackClient>,
        output_tx: mpsc::Sender<String>,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(Some(output_tx))),
        }
    }

    /// Create a handler with no output channel (responses are silently dropped).
    pub fn new_no_output(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<FallbackClient>,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(None)),
        }
    }

    /// Handle an inbound user message for a session.
    ///
    /// # Behaviour
    /// - **idle** → sets busy, spawns async LLM call, returns `LlmStarted`
    /// - **busy**  → enqueues message, returns `MessageQueued`
    ///
    /// When the LLM call finishes (success or failure), busy is cleared and any
    /// pending messages are drained recursively.
    pub async fn handle_message(&self, session_id: &str, content: String) -> HandleResult {
        if self.session_manager.is_session_busy(session_id).await {
            self.enqueue_pending(session_id, content).await;
            return HandleResult::MessageQueued;
        }

        // idle → start LLM call
        self.set_busy(session_id, true).await;
        let session_id = session_id.to_string();
        let content_for_task = content;
        let sm = Arc::clone(&self.session_manager);
        let fc = Arc::clone(&self.fallback_client);
        let output_tx = Arc::clone(&self.output_tx);

        tokio::spawn(async move {
            let result = Self::call_llm(&fc, &content_for_task).await;
            Self::finish_llm(&sm, &session_id, result, &fc, &output_tx).await;
        });

        HandleResult::LlmStarted
    }

    /// Set busy state on a conversation session.
    async fn set_busy(&self, session_id: &str, busy: bool) {
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            let cs = cs.write().await;
            cs.set_llm_busy(busy);
        }
    }

    /// Enqueue a pending message for a busy session.
    async fn enqueue_pending(&self, session_id: &str, content: String) {
        let msg = PendingMessage::new(
            format!("pending-{}", chrono::Utc::now().timestamp_millis()),
            content,
        );
        if let Err(e) = self
            .session_manager
            .push_pending_message(session_id, msg)
            .await
        {
            tracing::warn!(session_id, error = %e, "failed to enqueue pending message");
        }
    }

    /// Make a single non-streaming LLM call and return the text content.
    async fn call_llm(
        fallback_client: &Arc<FallbackClient>,
        content: &str,
    ) -> Result<String, crate::llm::LLMError> {
        let request = ChatRequest {
            model: String::new(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: content.to_string(),
            }],
            temperature: 0.7,
            max_tokens: None,
        };
        let response: ChatResponse = fallback_client.chat(request).await?;
        Ok(response.content)
    }

    /// Called when an LLM call finishes (success or failure).
    /// Clears busy flag, sends output, and drains pending messages.
    async fn finish_llm(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<String, crate::llm::LLMError>,
        fallback_client: &Arc<FallbackClient>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<String>>>>,
    ) {
        Self::clear_busy_and_send(session_manager, session_id, result, output_tx).await;
        Self::drain_pending_loop(session_manager, session_id, fallback_client, output_tx).await;
    }

    /// Clear busy flag and send output (called for each LLM completion).
    async fn clear_busy_and_send(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<String, crate::llm::LLMError>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<String>>>>,
    ) {
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            let cs = cs.write().await;
            cs.set_llm_busy(false);
        }

        match result {
            Ok(text) => {
                let guard = output_tx.read().await;
                if let Some(tx) = guard.as_ref() {
                    let _ = tx.send(text).await;
                }
            }
            Err(err) => {
                tracing::warn!(session_id, error = %err, "LLM call failed");
            }
        }
    }

    /// Drain pending messages until the queue is empty (loop-based, no recursion).
    async fn drain_pending_loop(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        fallback_client: &Arc<FallbackClient>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<String>>>>,
    ) {
        loop {
            // Get next pending message
            let Some(pending) = session_manager.pop_pending_message(session_id).await else {
                break;
            };

            // Set busy before calling LLM
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs = cs.write().await;
                cs.set_llm_busy(true);
            }

            let result = Self::call_llm(fallback_client, &pending.content).await;
            Self::clear_busy_and_send(session_manager, session_id, result, output_tx).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::fallback::FallbackClient;
    use crate::llm::LLMRegistry;

    fn handler_with_sm(sm: Arc<SessionManager>) -> SessionMessageHandler {
        let registry = Arc::new(LLMRegistry::new());
        let fallback = Arc::new(FallbackClient::from_strings(registry, vec![]));
        SessionMessageHandler::new_no_output(sm, fallback)
    }

    fn make_msg() -> crate::gateway::Message {
        use std::collections::HashMap;
        crate::gateway::Message {
            id: "msg_1".into(),
            from: "alice".into(),
            to: "bob".into(),
            content: "hello".into(),
            channel: "ch".into(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_idle_message_returns_llm_started() {
        let config = crate::gateway::GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: crate::gateway::DmScope::default(),
        };
        let sm = Arc::new(SessionManager::new(&config, None));
        let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
        let handler = handler_with_sm(Arc::clone(&sm));
        let result = handler.handle_message(&sid, "hello".to_string()).await;
        assert!(matches!(result, HandleResult::LlmStarted));
    }

    #[tokio::test]
    async fn test_busy_message_returns_queued() {
        let config = crate::gateway::GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: crate::gateway::DmScope::default(),
        };
        let sm = Arc::new(SessionManager::new(&config, None));
        let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

        // Manually set busy
        if let Some(cs) = sm.get_conversation_session(&sid).await {
            cs.write().await.set_llm_busy(true);
        }

        let handler = handler_with_sm(Arc::clone(&sm));
        let result = handler.handle_message(&sid, "hello".to_string()).await;
        assert!(matches!(result, HandleResult::MessageQueued));

        // Verify message was actually enqueued
        if let Some(pending) = sm.pop_pending_message(&sid).await {
            assert_eq!(pending.content, "hello");
        } else {
            panic!("expected pending message");
        }
    }

    #[tokio::test]
    async fn test_no_pending_no_recursion() {
        let config = crate::gateway::GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: crate::gateway::DmScope::default(),
        };
        let sm = Arc::new(SessionManager::new(&config, None));
        let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
        let handler = handler_with_sm(Arc::clone(&sm));

        // With empty fallback chain, call will fail — but we just verify it doesn't panic
        handler.handle_message(&sid, "hello".to_string()).await;
        // Give the task a moment to run
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // No pending messages exist
        assert!(sm.pop_pending_message(&sid).await.is_none());
    }

    /// After an LLM call completes (even with empty chain → failure),
    /// busy should be cleared so the session becomes idle again.
    #[tokio::test]
    async fn test_llm_failure_resets_busy() {
        let config = crate::gateway::GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: crate::gateway::DmScope::default(),
        };
        let sm = Arc::new(SessionManager::new(&config, None));
        let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
        let handler = handler_with_sm(Arc::clone(&sm));

        // Start a call — busy becomes true
        let result = handler.handle_message(&sid, "hello".to_string()).await;
        assert!(matches!(result, HandleResult::LlmStarted));
        assert!(
            sm.is_session_busy(&sid).await,
            "busy should be true immediately after call"
        );

        // Wait for the async LLM task to finish (it will fail because chain is empty)
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Busy should be cleared after LLM failure
        assert!(
            !sm.is_session_busy(&sid).await,
            "busy should be reset to false after LLM failure"
        );
    }

    /// After an LLM call completes, pending messages are automatically drained
    /// and the session handles them in order.
    #[tokio::test]
    async fn test_pending_consumed_after_llm_done() {
        let config = crate::gateway::GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: crate::gateway::DmScope::default(),
        };
        let sm = Arc::new(SessionManager::new(&config, None));
        let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
        let handler = handler_with_sm(Arc::clone(&sm));

        // First message starts LLM call, busy = true
        handler.handle_message(&sid, "first".to_string()).await;
        assert!(sm.is_session_busy(&sid).await);

        // Second message while busy → enqueued
        let result = handler.handle_message(&sid, "second".to_string()).await;
        assert!(matches!(result, HandleResult::MessageQueued));

        // Wait for first LLM call to finish and drain pending
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        // After drain: no more pending (the "second" message was consumed by drain loop)
        assert!(
            sm.pop_pending_message(&sid).await.is_none(),
            "pending message should have been consumed during drain"
        );
    }

    /// Multiple pending messages are consumed in FIFO order.
    #[tokio::test]
    async fn test_multiple_pending_fifo_order() {
        let config = crate::gateway::GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: crate::gateway::DmScope::default(),
        };
        let sm = Arc::new(SessionManager::new(&config, None));
        let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
        let handler = handler_with_sm(Arc::clone(&sm));

        // Start first LLM call
        handler.handle_message(&sid, "first".to_string()).await;

        // Enqueue two more while busy
        handler.handle_message(&sid, "second".to_string()).await;
        handler.handle_message(&sid, "third".to_string()).await;

        // Verify order by draining all pending and checking order
        let mut pending = Vec::new();
        while let Some(msg) = sm.pop_pending_message(&sid).await {
            pending.push(msg);
        }
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].content, "second");
        assert_eq!(pending[1].content, "third");

        // Wait for all LLM calls to finish (first + two drained)
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // All pending should have been drained
        assert!(sm.pop_pending_message(&sid).await.is_none());
    }
}
