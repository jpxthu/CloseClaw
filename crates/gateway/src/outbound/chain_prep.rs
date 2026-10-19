//! Outbound chain preparation shared by batch and streaming paths.

use crate::outbound_helpers::make_outbound_meta;
use crate::outbound_raw_log::OutboundRawLogSnapshot;
use crate::{Gateway, GatewayError};
use closeclaw_common::processor::ProcessedMessage;
use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::ContentBlock;

impl Gateway {
    /// Write the outbound raw-log snapshot, bypassing the full chain.
    ///
    /// The write goes through the composition-root-injected
    /// [`OutboundRawLogWriter`](crate::outbound_raw_log::OutboundRawLogWriter);
    /// without an injection (bypass / test construction) the write is skipped.
    ///
    /// On write failure the error is logged and the original content is
    /// forwarded unchanged (fail-open), so the message is never blocked
    /// by raw-log issues.
    pub(crate) async fn process_outbound_raw_log_only(
        &self,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        channel: &str,
    ) -> Result<ProcessedMessage, GatewayError> {
        let meta = make_outbound_meta(&[("channel", channel)]);
        let input = self.make_outbound_input(raw_output, content_blocks, meta);
        let Some(writer) = self.outbound_raw_log.as_ref() else {
            return Ok(input);
        };
        let snapshot = OutboundRawLogSnapshot {
            content: raw_output.to_string(),
            content_blocks: input.content_blocks.clone(),
            metadata: input.metadata.clone(),
        };
        if let Err(e) = writer.write(snapshot).await {
            tracing::error!(
                channel = %channel,
                error = %e,
                "outbound raw-log write failed, forwarding original content"
            );
        }
        Ok(input)
    }

    /// Run the outbound processor chain if configured, otherwise bypass.
    pub(crate) async fn process_or_bypass(
        &self,
        _raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        channel: &str,
        session_id: &str,
        verbosity_level: VerbosityLevel,
    ) -> Result<ProcessedMessage, GatewayError> {
        let meta = make_outbound_meta(&[
            ("channel", channel),
            ("session_id", session_id),
            ("verbosity_level", &verbosity_level.to_string()),
        ]);
        let input = self.make_outbound_input(_raw_output, content_blocks, meta);
        let Some(registry) = self.processor_registry.read().unwrap().clone() else {
            return Ok(input);
        };
        registry
            .process_outbound(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))
    }

    /// Build a [`ProcessedMessage`] from raw output.
    pub(super) fn make_outbound_input(
        &self,
        _raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        metadata: std::collections::HashMap<String, String>,
    ) -> ProcessedMessage {
        let blocks = if content_blocks.is_empty() {
            vec![ContentBlock::Text(_raw_output.to_string())]
        } else {
            content_blocks
        };
        ProcessedMessage {
            content_blocks: blocks,
            metadata,
        }
    }
}
