//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`](crate::im::IMPlugin) registry.

use super::{Gateway, GatewayError, Message};
use crate::im::IMPlugin;
use crate::llm::types::ContentBlock;
use crate::processor_chain::{DslParseResult, ProcessedMessage};
use crate::renderer::RenderedOutput;

/// Per-call context for dispatching a rendered output and persisting its
/// checkpoint. Bundled into a struct to keep the helper's parameter list short.
struct DispatchCtx<'a> {
    plugin: &'a std::sync::Arc<dyn IMPlugin>,
    rendered: &'a RenderedOutput,
    /// Plain-text fallback used when the rendered payload does not carry a
    /// `content.text` field. Typically the processed chain's `content`.
    fallback_text: &'a str,
    session_id: &'a str,
    channel: &'a str,
    chat_id: String,
}

impl Gateway {
    /// Send an outbound message (agent response) via the registered IM plugin.
    ///
    /// Flow (single path; legacy renderer / processor-only / bypass branches
    /// are unified here):
    /// 1. Resolve `chat_id` from `session_id` via `SessionManager::get_chat_id`.
    /// 2. Resolve the [`IMPlugin`](super::im::IMPlugin) registered for `channel`
    ///    through `self.plugins`.
    /// 3. Run the processor chain (if `processor_registry` is configured) to
    ///    produce a [`ProcessedMessage`]; otherwise bypass with a synthetic
    ///    `ProcessedMessage` wrapping the raw input.
    /// 4. Honor `processed.suppress` — return `Ok(())` without sending.
    /// 5. Extract `dsl_result` from `processed.metadata["dsl_result"]` (stored
    ///    as a JSON-encoded string by the DSL processor).
    /// 6. Call `plugin.render(blocks, dsl_result)` to obtain a
    ///    [`RenderedOutput`](crate::renderer::RenderedOutput); fall back to a
    ///    single `ContentBlock::Text` block when `content_blocks` is empty.
    /// 7. Dispatch by `msg_type` (`"text"` / `"interactive"`) through
    ///    `plugin.send`. Any other type is an [`GatewayError::OutboundError`].
    /// 8. After each successful send, trigger checkpoint persistence.
    /// 9. `thread_id` is not yet wired through — `None` is passed to
    ///    `plugin.send`.
    pub async fn send_outbound(
        &self,
        session_id: &str,
        channel: &str,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
    ) -> Result<(), GatewayError> {
        // 1. Resolve chat_id and plugin.
        let chat_id = self
            .session_manager
            .get_chat_id(session_id)
            .await
            .ok_or(GatewayError::MissingSessionId)?;
        let plugin = self
            .get_plugin(channel)
            .await
            .ok_or_else(|| GatewayError::UnknownChannel(channel.to_string()))?;

        // 2. Run processor chain (or bypass) and honor suppress.
        let processed = self.process_or_bypass(raw_output, content_blocks).await?;
        if processed.suppress {
            return Ok(());
        }

        // 3. Extract dsl_result (serialized as a JSON string by DslParser).
        let dsl_result: Option<DslParseResult> = processed
            .metadata
            .get("dsl_result")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str(s).ok());

        // 4. Render via the plugin; fall back to a single Text block when
        // content_blocks is empty.
        let rendered = {
            let owned_fallback;
            let blocks: &[ContentBlock] = if processed.content_blocks.is_empty() {
                owned_fallback = vec![ContentBlock::Text(processed.content.clone())];
                &owned_fallback
            } else {
                &processed.content_blocks
            };
            plugin.render(blocks, dsl_result.as_ref())
        };

        // 5. Dispatch by msg_type and persist checkpoint on success.
        self.dispatch_and_persist(DispatchCtx {
            plugin: &plugin,
            rendered: &rendered,
            fallback_text: &processed.content,
            session_id,
            channel,
            chat_id,
        })
        .await
    }

    /// Dispatch a rendered output to its destination plugin and persist the
    /// outbound checkpoint. `msg_type` drives the dispatch:
    /// - `"text"`: extract text from `rendered.payload`, build a [`Message`],
    ///   call `plugin.send`.
    /// - `"interactive"`: call `plugin.send` directly, build a [`Message`]
    ///   from the serialized payload for checkpointing.
    /// - any other: return [`GatewayError::OutboundError`].
    async fn dispatch_and_persist(&self, ctx: DispatchCtx<'_>) -> Result<(), GatewayError> {
        match ctx.rendered.msg_type.as_str() {
            "text" => {
                let text = ctx
                    .rendered
                    .payload
                    .get("content")
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(ctx.fallback_text)
                    .to_string();
                let msg = Self::make_outbound_msg(ctx.channel, ctx.chat_id.clone(), text);
                ctx.plugin.send(ctx.rendered, &ctx.chat_id, None).await?;
                self.persist_outbound_checkpoint(ctx.session_id, &msg).await;
                Ok(())
            }
            "interactive" => {
                let payload_str = serde_json::to_string(&ctx.rendered.payload)
                    .unwrap_or_else(|_| "{}".to_string());
                ctx.plugin.send(ctx.rendered, &ctx.chat_id, None).await?;
                let msg = Self::make_outbound_msg(ctx.channel, ctx.chat_id, payload_str);
                self.persist_outbound_checkpoint(ctx.session_id, &msg).await;
                Ok(())
            }
            _ => Err(GatewayError::OutboundError(format!(
                "unknown msg_type: {}",
                ctx.rendered.msg_type
            ))),
        }
    }

    /// Run the outbound processor chain if configured, otherwise bypass with
    /// a synthetic [`ProcessedMessage`] wrapping the raw input.
    async fn process_or_bypass(
        &self,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
    ) -> Result<ProcessedMessage, GatewayError> {
        let Some(ref registry) = self.processor_registry else {
            return Ok(ProcessedMessage {
                content: raw_output.to_string(),
                metadata: serde_json::Map::new(),
                suppress: false,
                content_blocks,
            });
        };
        let input = ProcessedMessage {
            content: raw_output.to_string(),
            metadata: serde_json::Map::new(),
            suppress: false,
            content_blocks,
        };
        registry
            .process_outbound(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))
    }

    /// Build a [`Message`] for checkpoint persistence from outbound fields.
    fn make_outbound_msg(channel: &str, to: String, content: String) -> Message {
        Message {
            id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
            from: "agent".to_string(),
            to,
            content,
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Persist outbound message to checkpoint if checkpoint_manager is configured.
    async fn persist_outbound_checkpoint(&self, session_id: &str, msg: &Message) {
        let Some(ref cm) = self.checkpoint_manager else {
            return;
        };
        let checkpoint = match cm.load(session_id).await {
            Ok(Some(cp)) => cp,
            Ok(None) => crate::session::persistence::SessionCheckpoint::new(session_id.to_string()),
            Err(e) => {
                tracing::warn!(session_id, "failed to load checkpoint: {}", e);
                return;
            }
        };
        let mut pending =
            crate::session::persistence::PendingMessage::new(msg.id.clone(), msg.content.clone());
        pending.mark_sent();
        let mut cp = checkpoint.add_pending_message(pending);
        // Sync per-session append-section list from ConversationSession
        // (issue #860: archived session restore preserves append content).
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            let cs = cs.read().await;
            cp.system_appends = cs.system_appends().to_vec();
        }
        cp.touch();
        if let Err(e) = cm.save(cp).await {
            tracing::warn!(session_id, "failed to save checkpoint: {}", e);
        }
    }
}
