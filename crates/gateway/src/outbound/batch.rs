//! Batch-mode outbound entry: resolve session dispatch fields, run the
//! full batch processor chain (VerbosityFilter → DslParser →
//! OutboundRawLog), render, then hand off to the dispatch pipeline
//! ([`batch_dispatch`]) or the plain-text fallbacks ([`batch_fallback`]).

use crate::{Gateway, GatewayError};
use closeclaw_common::processor::DslParseResult;
use closeclaw_llm::types::ContentBlock;

use super::batch_dispatch::DispatchCtx;
use super::types::{SendOutboundIds, SendOutcome};

impl Gateway {
    /// Send an outbound message (agent response) via the registered IM plugin.
    ///
    /// Flow: resolve chat_id + plugin → resolve VerbosityLevel → run full batch
    /// outbound chain (VerbosityFilter → DslParser → OutboundRawLog) → render →
    /// dispatch by msg_type → persist checkpoint.
    ///
    /// Streaming mode also runs VerbosityFilter — during the incremental phase
    /// at block boundaries — and skips it in the finish phase (see
    /// `finish_streaming_pipeline`).
    pub async fn send_outbound(
        &self,
        session_id: &str,
        channel: &str,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        ids: SendOutboundIds,
    ) -> Result<SendOutcome, GatewayError> {
        let SendOutboundIds {
            trace_id,
            session_key,
        } = ids;
        // 1. Resolve chat_id and plugin.
        let chat_id = self.resolve_outbound_chat_id(session_id).await?;
        let Some(plugin) = self.get_plugin(channel).await else {
            return self
                .fallback_to_plain_text(channel, raw_output)
                .await
                .map(|()| SendOutcome::Sent);
        };

        // 2. Resolve verbosity level and inject into chain metadata.
        let verbosity_level = self.resolve_outbound_verbosity_level(session_id).await;

        // 3. Full batch outbound chain (VerbosityFilter → DslParser → OutboundRawLog).
        let processed = self
            .process_or_bypass(
                raw_output,
                content_blocks,
                channel,
                session_id,
                verbosity_level,
            )
            .await?;
        if processed.content_blocks.is_empty() {
            return Ok(SendOutcome::Sent);
        }

        let blocks = &processed.content_blocks;
        let dsl_result: Option<DslParseResult> = processed
            .metadata
            .get("dsl_result")
            .and_then(|s| serde_json::from_str(s).ok());
        let render_start = std::time::Instant::now();
        let rendered = plugin.render(blocks, dsl_result.as_ref());
        if channel == "feishu" {
            let render_duration_ms = render_start.elapsed().as_millis() as u64;
            crate::outbound_helpers::emit_feishu_render_event(
                self,
                trace_id.as_deref().unwrap_or(""),
                session_key.as_deref(),
                channel,
                render_duration_ms,
                None, // outbound render event, no parent context needed
            );
        }
        let routing = self.resolve_outbound_thread_routing(session_id).await;
        let fallback_text = blocks
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .unwrap_or("");
        self.dispatch_and_persist(DispatchCtx {
            plugin: &plugin,
            rendered: &rendered,
            fallback_text,
            session_id,
            channel,
            chat_id,
            thread_id: routing.thread_id,
            reply_ref: routing.reply_ref,
            dsl_result: processed.metadata.get("dsl_result").cloned(),
            content_blocks: serde_json::to_string(&processed.content_blocks).ok(),
            trace_id,
            session_key,
        })
        .await
    }
}
