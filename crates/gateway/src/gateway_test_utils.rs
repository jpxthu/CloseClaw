//! Shared test utilities for the workflow / outbound gateway tests.
//!
//! Contains the outbound-capturing [`SpyPlugin`] plus the inbound routing
//! helpers [`routed_msg`] / [`empty_dbg`], shared by `workflow_owner/tests.rs`,
//! `slash_workflow_start_tests.rs`, `slash_workflow_stop_tests.rs`,
//! `workflow_integration_tests.rs`, and `slash_permission_outbound_tests.rs`
//! (previously five verbatim copies).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::inbound_queue::InboundDebugCtx;
use closeclaw_common::im_plugin::{AdapterError, MessageType, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, ProcessedMessage};
use closeclaw_common::IMPlugin;

/// Spy plugin capturing the text of every outbound reply.
///
/// `render` joins all text blocks; `send` records the rendered text so
/// tests can assert on the exact reply content.
///
/// Capturing the `send` text alone is sufficient to assert on either
/// outbound path: both `send_system_notification` and
/// `send_outbound_simplified` ultimately reach the plugin's `send`
/// method.
pub(crate) struct SpyPlugin {
    messages: Arc<Mutex<Vec<String>>>,
}

impl SpyPlugin {
    pub(crate) fn new(messages: Arc<Mutex<Vec<String>>>) -> Self {
        Self { messages }
    }
}

#[async_trait]
impl IMPlugin for SpyPlugin {
    fn platform(&self) -> &str {
        "mock"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_owned();
        if !text.is_empty() {
            self.messages.lock().expect("spy poisoned").push(text);
        }
        Ok(())
    }
}

/// Build a [`ProcessedMessage`] carrying a single text block and the
/// routing metadata expected by `route_and_dispatch`.
pub(crate) fn routed_msg(text: &str, peer: &str) -> ProcessedMessage {
    let mut m = HashMap::new();
    m.insert("peer_id".into(), peer.into());
    m.insert(
        "message_type".into(),
        serde_json::to_string(&MessageType::Text).unwrap(),
    );
    m.insert("session_key".into(), "k".into());
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(text.into())],
        metadata: m,
    }
}

/// Debug context without tracing information.
pub(crate) fn empty_dbg() -> InboundDebugCtx<'static> {
    InboundDebugCtx {
        trace_id: None,
        session_key: None,
        root_ctx: None,
    }
}
