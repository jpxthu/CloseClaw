//! Streaming finalize phase: post-stream pipeline that selects the
//! content blocks, runs the Processor Chain finish pass (DslParser →
//! OutboundRawLog, skipping VerbosityFilter), builds the final
//! [`StreamResult`], and persists the outbound checkpoint.

use crate::outbound_helpers::{make_outbound_meta, merge_dsl_results, CheckpointMeta, StreamState};
use crate::{Gateway, GatewayError};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::{ContentBlock, UnifiedUsage};

use super::types::StreamResult;

/// Build the final [`StreamResult`] shared by the registry-missing
/// early-return and the normal pipeline paths of
/// [`Gateway::finish_streaming_pipeline`].
fn build_stream_result(
    content_blocks: Vec<ContentBlock>,
    dsl_result: Option<String>,
    usage_override: Option<UnifiedUsage>,
    state: &StreamState,
) -> StreamResult {
    StreamResult {
        content_blocks,
        usage: usage_override.unwrap_or_else(|| state.usage.clone()),
        retry_attempts: 0,
        dsl_result,
        // Attached by the production call site from the
        // invocation's returned target.
        call_target: None,
    }
}

impl Gateway {
    /// Post-stream pipeline: select content blocks, run the
    /// Processor Chain (DslParser → OutboundRawLog), and build the
    /// final [`StreamResult`].
    ///
    /// VerbosityFilter is skipped here — it already ran during the
    /// incremental phase via [`ProcessorChain::process_outbound_incremental`].
    pub(super) async fn finish_streaming_pipeline(
        &self,
        session_blocks: Option<(Vec<ContentBlock>, Option<UnifiedUsage>)>,
        mut state: StreamState,
        channel: &str,
        session_id: &str,
        verbosity_level: VerbosityLevel,
    ) -> Result<StreamResult, GatewayError> {
        let (content_blocks_for_pipeline, usage_override) = match session_blocks {
            Some((blocks, usage)) => (blocks, usage),
            None => (std::mem::take(&mut state.content_blocks), None),
        };

        // Run the outbound Processor Chain (DslParser → OutboundRawLog),
        // skipping VerbosityFilter which already ran during the incremental
        // phase via process_outbound_incremental.
        let meta = make_outbound_meta(&[
            ("channel", channel),
            ("session_id", session_id),
            ("verbosity_level", &verbosity_level.to_string()),
        ]);
        let dsl_result_incremental = state.dsl_instructions.clone();
        let Some(registry) = self.processor_registry.read().unwrap().clone() else {
            let dsl_result = if dsl_result_incremental.is_empty() {
                None
            } else {
                let r = DslParseResult {
                    instructions: dsl_result_incremental,
                };
                serde_json::to_string(&r).ok()
            };
            return Ok(build_stream_result(
                content_blocks_for_pipeline,
                dsl_result,
                usage_override,
                &state,
            ));
        };
        let input = self.make_outbound_input("", content_blocks_for_pipeline, meta);
        let processed = registry
            .process_outbound_without_verbosity(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))?;

        let dsl_result = merge_dsl_results(&processed.metadata, dsl_result_incremental);

        Ok(build_stream_result(
            processed.content_blocks,
            dsl_result,
            usage_override,
            &state,
        ))
    }
}

impl Gateway {
    /// Persist the streaming outbound checkpoint — mirrors the batch path
    /// in `dispatch_and_persist`. Without this, a crash after streaming
    /// completes would lose the outbound history.
    pub(super) async fn persist_streaming_completion_checkpoint(
        &self,
        channel: &str,
        session_id: &str,
        chat_id: &str,
        result: &StreamResult,
    ) {
        let text = result
            .content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        let content_blocks_json = serde_json::to_string(&result.content_blocks).unwrap_or_default();
        let msg_id = format!("out-{}", chrono::Utc::now().timestamp_millis());
        let meta = CheckpointMeta {
            platform: Some(channel.to_string()),
            dsl_result: result.dsl_result.clone(),
            content_blocks: Some(content_blocks_json),
        };
        let msg = Self::make_outbound_msg(channel, chat_id.to_string(), msg_id, text, meta);
        crate::outbound_helpers::persist_outbound_checkpoint(self, session_id, &msg, true).await;
    }
}

#[cfg(test)]
#[path = "streaming_finalize_tests.rs"]
mod streaming_finalize_tests;
