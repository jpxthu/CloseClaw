//! Shared test support for system-notification delivery tests.
//!
//! [`RecordingPlugin`] (platform "mock") records every non-empty
//! delivered text so delivery/drop assertions can inspect what reached
//! the plugin. Shared by `outbound_system_notification_tests` and
//! `announce_drain_audience_tests` (previously near-duplicate local
//! definitions); the integration crate keeps its independent copy
//! (platform "tracking").

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, DslParseResult};

/// Spy plugin (platform "mock") recording every delivered text.
pub(crate) struct RecordingPlugin {
    messages: Arc<Mutex<Vec<String>>>,
}

impl RecordingPlugin {
    pub(crate) fn new(messages: Arc<Mutex<Vec<String>>>) -> Self {
        Self { messages }
    }
}

#[async_trait]
impl IMPlugin for RecordingPlugin {
    fn platform(&self) -> &str {
        "mock"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
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
