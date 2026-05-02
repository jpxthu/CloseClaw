//! Processor module - message processor registry and traits.
//!
//! Architecture:
//! - `mod.rs` — `ProcessPhase`, `MessageProcessor` trait, `MessageContext`,
//!   `ProcessError`, `ProcessorRegistry`, `ProcessedMessage`
//! - `cleaner.rs` — `FeishuMessageCleaner` (inbound processor) + cleaning logic
//! - `session_router.rs` — `SessionRouter` (inbound, priority 20) + session routing

mod cleaner;
mod session_router;

use async_trait::async_trait;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use crate::gateway::SessionManager;
use crate::processor_chain::{DslInstruction, DslParseResult};

pub use cleaner::FeishuMessageCleaner;
use serde_json::Value;
pub use session_router::SessionRouter;

// ---------------------------------------------------------------------------
// ProcessPhase
// ---------------------------------------------------------------------------

/// Phase of a message processor — determines when it runs in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProcessPhase {
    /// Runs on incoming webhook events (inbound).
    Inbound,
    /// Runs on outbound LLM-generated content (outbound).
    Outbound,
}

// ---------------------------------------------------------------------------
// MessageProcessor trait
// ---------------------------------------------------------------------------

/// Trait for message processors that transform messages in a pipeline.
///
/// Each processor declares its `priority` (lower = runs earlier) and `phase`.
#[async_trait]
pub trait MessageProcessor: Send + Sync {
    /// Return the processor priority. Lower values run earlier.
    fn priority(&self) -> i32;

    /// Return the processing phase.
    fn phase(&self) -> ProcessPhase;

    /// Process the message and return the result.
    async fn process(
        &self,
        ctx: &MessageContext,
        msg: &Value,
    ) -> Result<ProcessedMessage, ProcessError>;
}

// ---------------------------------------------------------------------------
// MessageContext
// ---------------------------------------------------------------------------

/// Context passed to processors during processing.
#[derive(Debug, Clone, Default)]
pub struct MessageContext {
    /// Additional metadata accumulated by earlier processors.
    pub metadata: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// ProcessError
// ---------------------------------------------------------------------------

/// Errors that can occur during message processing.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("Missing 'message' field in webhook payload")]
    MissingMessage,

    #[error("Unsupported message type: {0}")]
    UnsupportedMessageType(String),

    #[error("Processing failed: {0}")]
    ProcessingFailed(String),

    #[error("Registry error: {0}")]
    RegistryError(String),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Session not supported for channel: {0}")]
    SessionNotSupportedForChannel(String),
}

// ---------------------------------------------------------------------------
// ProcessedMessage
// ---------------------------------------------------------------------------

/// Result of cleaning a raw feishu webhook event.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProcessedMessage {
    /// Cleaned message content.
    pub content: String,
    /// Additional metadata (only `chat_type` when `group`).
    pub metadata: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// ProcessorRegistry
// ---------------------------------------------------------------------------

/// Registry of message processors, grouped by phase and sorted by priority.
#[derive(Default)]
pub struct ProcessorRegistry {
    inbound: BTreeMap<i32, Vec<Arc<dyn MessageProcessor>>>,
    outbound: BTreeMap<i32, Vec<Arc<dyn MessageProcessor>>>,
}

impl ProcessorRegistry {
    /// Create a new registry with default processors registered:
    /// - [`SessionRouter`] (inbound, priority 20) — resolves session IDs
    /// - [`FeishuMessageCleaner`] (inbound, priority 30)
    /// - [`FeishuMessageCleaner`] (inbound, priority 30)
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        let mut registry = Self::default();
        registry.register(SessionRouter::new(session_manager));
        registry.register(FeishuMessageCleaner);
        registry
    }

    /// Register a processor. Panic on duplicate priority in the same phase.
    pub fn register<P: MessageProcessor + 'static>(&mut self, processor: P) {
        let phase = processor.phase();
        let priority = processor.priority();
        let arc: Arc<dyn MessageProcessor> = Arc::new(processor);
        match phase {
            ProcessPhase::Inbound => self.inbound.entry(priority).or_default().push(arc),
            ProcessPhase::Outbound => self.outbound.entry(priority).or_default().push(arc),
        }
    }

    /// Process a message through all inbound processors in priority order.
    ///
    /// Each processor's output (`ProcessedMessage`) is serialized to JSON and
    /// fed as the input `Value` to the next processor in the chain. The final
    /// result is returned.
    pub async fn process_inbound(&self, msg: &Value) -> Result<ProcessedMessage, ProcessError> {
        let mut ctx = MessageContext::default();
        let mut current: Value = msg.clone();

        let mut inbound_iter = self.inbound.values();
        if inbound_iter.next().is_none() {
            return Err(ProcessError::RegistryError(
                "No inbound processors registered".into(),
            ));
        }
        drop(inbound_iter);

        for processors in self.inbound.values() {
            for processor in processors {
                let result = processor.process(&ctx, &current).await?;
                ctx.metadata.extend(result.metadata.clone());
                current = serde_json::to_value(&result)?;
            }
        }

        serde_json::from_value(current)
            .map_err(|e| ProcessError::ProcessingFailed(format!("final inbound result: {}", e)))
    }

    /// Process a message through all outbound processors in priority order.
    ///
    /// Each processor's output (`ProcessedMessage`) is serialized to JSON and
    /// fed as the input `Value` to the next processor in the chain. The final
    /// result is returned.
    pub async fn process_outbound(&self, msg: &Value) -> Result<ProcessedMessage, ProcessError> {
        let mut ctx = MessageContext::default();
        let mut current: Value = msg.clone();

        let mut outbound_iter = self.outbound.values();
        if outbound_iter.next().is_none() {
            return Err(ProcessError::RegistryError(
                "No outbound processors registered".into(),
            ));
        }
        drop(outbound_iter);

        for processors in self.outbound.values() {
            for processor in processors {
                let result = processor.process(&ctx, &current).await?;
                ctx.metadata.extend(result.metadata.clone());
                current = serde_json::to_value(&result)?;
            }
        }

        serde_json::from_value(current)
            .map_err(|e| ProcessError::ProcessingFailed(format!("final outbound result: {}", e)))
    }
}

impl Debug for ProcessorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessorRegistry").finish()
    }
}

// ---------------------------------------------------------------------------
// clean_feishu_message — public entry point (backward compatibility)
// ---------------------------------------------------------------------------

/// Entry point — parses the raw webhook JSON and dispatches to FeishuMessageCleaner.
pub async fn clean_feishu_message(raw: &Value) -> ProcessedMessage {
    let cleaner = FeishuMessageCleaner;
    let ctx = MessageContext::default();
    cleaner
        .process(&ctx, raw)
        .await
        .unwrap_or_else(|_e| ProcessedMessage {
            content: String::new(),
            metadata: BTreeMap::new(),
        })
}

// ---------------------------------------------------------------------------
// ProcessorRegistry unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_session_manager() -> Arc<SessionManager> {
        Arc::new(SessionManager::new(
            &crate::gateway::GatewayConfig {
                name: "test".to_string(),
                rate_limit_per_minute: 100,
                max_message_size: 65536,
                dm_scope: crate::gateway::DmScope::PerChannelPeer,
            },
            None,
        ))
    }

    /// A no-op inbound processor that records how many times it was called
    /// and at what priority.
    struct SpyProcessor {
        priority: i32,
        phase: ProcessPhase,
        call_count: Arc<AtomicUsize>,
        content: String,
    }

    impl SpyProcessor {
        fn new(
            priority: i32,
            phase: ProcessPhase,
            call_count: Arc<AtomicUsize>,
            content: &str,
        ) -> Self {
            Self {
                priority,
                phase,
                call_count,
                content: content.to_string(),
            }
        }
    }

    #[async_trait]
    impl MessageProcessor for SpyProcessor {
        fn priority(&self) -> i32 {
            self.priority
        }
        fn phase(&self) -> ProcessPhase {
            self.phase
        }
        async fn process(
            &self,
            _ctx: &MessageContext,
            _msg: &Value,
        ) -> Result<ProcessedMessage, ProcessError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(ProcessedMessage {
                content: self.content.clone(),
                metadata: BTreeMap::new(),
            })
        }
    }

    // --- Empty registry ---

    fn empty_registry() -> ProcessorRegistry {
        ProcessorRegistry {
            inbound: Default::default(),
            outbound: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_empty_registry_inbound_error() {
        let registry = empty_registry();
        let result = registry.process_inbound(&serde_json::json!({})).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("No inbound processors"),
            "unexpected error: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_empty_registry_outbound_error() {
        let registry = empty_registry();
        let result = registry
            .process_outbound(&serde_json::json!({"content": "test"}))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("No outbound processors"),
            "unexpected error: {}",
            err
        );
    }

    // --- Registration and priority ordering ---

    #[tokio::test]
    async fn test_registry_default_has_all_processors() {
        let mgr = test_session_manager();
        let registry = ProcessorRegistry::new(mgr);
        // FeishuMessageCleaner (inbound, prio 30)
        assert!(!registry.inbound.is_empty());
    }

    #[tokio::test]
    async fn test_processors_called_in_priority_order() {
        // Shared counter to verify all three processors run (in order)
        let call_order = Arc::new(AtomicUsize::new(0));
        let mut registry = empty_registry();

        // Register three inbound processors with different priorities (ascending)
        let call0 = call_order.clone();
        registry.register(SpyProcessor::new(10, ProcessPhase::Inbound, call0, "first"));
        let call1 = call_order.clone();
        registry.register(SpyProcessor::new(
            20,
            ProcessPhase::Inbound,
            call1,
            "second",
        ));
        let call2 = call_order.clone();
        registry.register(SpyProcessor::new(30, ProcessPhase::Inbound, call2, "third"));

        let msg = serde_json::json!({
            "message": {
                "message_type": "text",
                "content": "{}"
            }
        });

        // Inbound chain: 10 -> 20 -> 30 (ascending priority)
        // Each processor's output becomes the next processor's input.
        // Final ProcessedMessage.content should be "third" (last in chain).
        let result = registry.process_inbound(&msg).await.unwrap();
        assert_eq!(result.content, "third");
    }

    // --- Inbound chain end-to-end with fixture data ---

    fn feishu_fixtures_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/feishu")
    }

    fn load_raw_fixture(filename: &str) -> Value {
        let path = feishu_fixtures_dir().join(filename);
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn load_expected_fixture(filename: &str) -> ProcessedMessage {
        let path = feishu_fixtures_dir().join(filename);
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn test_inbound_chain_simple_text() {
        let mgr = test_session_manager();
        let registry = ProcessorRegistry::new(mgr);
        let raw =
            load_raw_fixture("im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json");
        let expected = load_expected_fixture("expected/01_text_simple.json");

        let result = registry.process_inbound(&raw).await.unwrap();
        assert_eq!(result.content, expected.content);
    }

    #[tokio::test]
    async fn test_inbound_chain_post_lists() {
        let mgr = test_session_manager();
        let registry = ProcessorRegistry::new(mgr);
        let raw =
            load_raw_fixture("im-message-receive_v1-no-event-id-2026-04-27T02-58-21-195Z.json");
        let expected = load_expected_fixture("expected/03_post_lists.json");

        let result = registry.process_inbound(&raw).await.unwrap();
        assert_eq!(result.content, expected.content);
    }

}
