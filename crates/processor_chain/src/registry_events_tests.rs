//! Tests for [`ProcessorRegistry`] chain debug-log events — `chain.routing`
//! (inbound) and `chain.outbound` emission, with and without a metadata
//! `session_key`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_common::processor::ContentBlock;
use closeclaw_debug_log::test_utils::{make_debug_log, wait_for_event_type};
use tempfile::TempDir;

use crate::processor_chain::context::MessageContext;
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use crate::processor_chain::registry::ProcessorRegistry;
use crate::ProcessedMessage;

/// Processor that injects a fixed `session_key` into the metadata map
/// (no entry when `None`); registrable on either chain.
struct SessionKeyProc {
    phase: ProcessPhase,
    session_key: Option<&'static str>,
}

impl SessionKeyProc {
    fn inbound(session_key: Option<&'static str>) -> Arc<Self> {
        Arc::new(Self {
            phase: ProcessPhase::Inbound,
            session_key,
        })
    }
    fn outbound(session_key: Option<&'static str>) -> Arc<Self> {
        Arc::new(Self {
            phase: ProcessPhase::Outbound,
            session_key,
        })
    }
}

#[async_trait]
impl MessageProcessor for SessionKeyProc {
    fn name(&self) -> &str {
        "session_key_proc"
    }
    fn phase(&self) -> ProcessPhase {
        self.phase
    }
    fn priority(&self) -> u8 {
        1
    }
    async fn process(
        &self,
        _ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        let mut metadata = HashMap::new();
        if let Some(key) = self.session_key {
            metadata.insert("session_key".to_string(), key.to_string());
        }
        Ok(Some(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("session_key_proc".to_string())],
            metadata,
        }))
    }
}

async fn registry_with_debug_log(dir: &TempDir) -> ProcessorRegistry {
    let debug_log = make_debug_log(dir).await;
    ProcessorRegistry::new()
        .with_debug_log(debug_log)
        .with_trace_id("registry-test-trace".to_string())
}

fn make_normalized(content: &str) -> NormalizedMessage {
    NormalizedMessage {
        platform: "test".to_string(),
        sender_id: "user_1".to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

#[tokio::test]
async fn test_chain_routing_event_carries_session_key() {
    let dir = TempDir::new().unwrap();
    let mut registry = registry_with_debug_log(&dir).await;
    registry.register(SessionKeyProc::inbound(Some("sess-1")));

    registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();

    let events = wait_for_event_type(dir.path(), "chain.routing").await;
    assert_eq!(
        events.len(),
        1,
        "expected exactly one chain.routing event, got {}",
        events.len()
    );
    assert_eq!(
        events[0].session_key.as_deref(),
        Some("sess-1"),
        "chain.routing event must carry the metadata session_key"
    );
    assert_eq!(events[0].payload["session_key"], "sess-1");
}

#[tokio::test]
async fn test_chain_routing_event_emitted_when_session_key_missing() {
    let dir = TempDir::new().unwrap();
    let mut registry = registry_with_debug_log(&dir).await;
    registry.register(SessionKeyProc::inbound(None));

    registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();

    let events = wait_for_event_type(dir.path(), "chain.routing").await;
    assert_eq!(
        events.len(),
        1,
        "chain.routing must still be emitted when session_key is absent, got {}",
        events.len()
    );
    assert_eq!(
        events[0].session_key, None,
        "missing metadata session_key must surface as None, not halt emission"
    );
    assert_eq!(events[0].payload["session_key"], serde_json::Value::Null);
}

#[tokio::test]
async fn test_chain_outbound_event_carries_session_key() {
    let dir = TempDir::new().unwrap();
    let mut registry = registry_with_debug_log(&dir).await;
    // Registering a processor is solely to avoid the empty-outbound-chain
    // early return (`process_outbound_filtered` short-circuits on
    // `is_empty`); the asserted `session_key` comes from the input metadata.
    registry.register(SessionKeyProc::outbound(None));

    let mut metadata = HashMap::new();
    metadata.insert("session_key".to_string(), "sess-2".to_string());
    registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("llm".to_string())],
            metadata,
        })
        .await
        .unwrap();

    let events = wait_for_event_type(dir.path(), "chain.outbound").await;
    assert_eq!(
        events.len(),
        1,
        "expected exactly one chain.outbound event, got {}",
        events.len()
    );
    assert_eq!(
        events[0].session_key.as_deref(),
        Some("sess-2"),
        "chain.outbound event must carry the metadata session_key"
    );
}

#[tokio::test]
async fn test_chain_outbound_event_emitted_when_session_key_missing() {
    let dir = TempDir::new().unwrap();
    let mut registry = registry_with_debug_log(&dir).await;
    // Registering a processor is solely to avoid the empty-outbound-chain
    // early return (`process_outbound_filtered` short-circuits on
    // `is_empty`); the asserted `session_key` comes from the input metadata.
    registry.register(SessionKeyProc::outbound(None));

    registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("llm".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    let events = wait_for_event_type(dir.path(), "chain.outbound").await;
    assert_eq!(
        events.len(),
        1,
        "chain.outbound must still be emitted when session_key is absent, got {}",
        events.len()
    );
    assert_eq!(
        events[0].session_key, None,
        "missing metadata session_key must surface as None, not halt emission"
    );
}
