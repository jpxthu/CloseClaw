//! Message routing and inbound chain processing for the Gateway.
//!
//! Extracted from `lib.rs` to keep the main file under the 1000-line limit.

use super::Gateway;
use crate::debug_log_emitter;
use crate::types::{GatewayError, Message};
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::processor::ProcessedMessage;
use closeclaw_debug_log::LogLevel;
use std::collections::HashMap;

/// 会话路由失败，请重试
const ROUTE_FAILED_MSG: &str =
    "\u{4F1A}\u{8BDD}\u{8DEF}\u{7531}\u{5931}\u{8D25}\u{FF0C}\u{8BF7}\u{91CD}\u{8BD5}";

impl Gateway {
    /// Forward a resolved message to the IM plugin for the given channel.
    pub(crate) async fn forward_to_plugin(
        &self,
        channel: &str,
        message: &Message,
        session_id: &str,
    ) -> Result<(), GatewayError> {
        if !self.session_manager.has_session(session_id).await {
            return Err(GatewayError::MissingSessionId);
        }
        let plugin = self
            .get_plugin(channel)
            .await
            .ok_or(GatewayError::UnknownChannel(channel.to_string()))?;
        if message.content.len() > self.config.max_message_size {
            return Err(GatewayError::MessageTooLarge);
        }
        let thread_id = self.session_manager.get_thread_id(session_id).await;
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": &message.content}}),
        };
        plugin
            .send(&output, &message.to, thread_id.as_deref())
            .await
            .map_err(|e| GatewayError::AdapterError(e.to_string()))
    }

    /// Send a best-effort user-visible error via the plugin.
    pub(crate) async fn send_user_error(&self, channel: &str, message: &Message) {
        if let Some(plugin) = self.get_plugin(channel).await {
            let err_output = RenderedOutput {
                msg_type: "text".into(),
                payload: serde_json::json!({
                    "content": {
                        "text":
                            ROUTE_FAILED_MSG
                    }
                }),
            };
            let _ = plugin.send(&err_output, &message.to, None).await;
        }
    }

    /// Emit a `route.decision` debug log entry for the given trace context.
    fn emit_route_debug_log(
        &self,
        trace_id: Option<&str>,
        session_key: Option<&str>,
        session_id: &str,
    ) {
        if let Some(tid) = trace_id {
            let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
            debug_log_emitter::emit_debug_event(
                guard.as_ref(),
                tid,
                session_key,
                LogLevel::Info,
                "gateway",
                "route.decision",
                serde_json::json!({
                    "session_key": session_key.unwrap_or_default(),
                    "session_id": session_id,
                }),
            );
        }
    }

    /// Resolve and forward via `session_key` (new path).
    ///
    /// Calls `SessionManager::resolve()`, sends any pending restore
    /// notification, then forwards to the plugin.
    async fn route_via_session_key(
        &self,
        channel: &str,
        message: &Message,
        session_key: &str,
        account_id: Option<&str>,
    ) -> Result<(), GatewayError> {
        let session_id = self
            .session_manager
            .resolve(session_key, channel, message, account_id)
            .await
            .map_err(|e| GatewayError::AdapterError(e.to_string()))?;

        self.emit_route_debug_log(
            message.metadata.get("trace_id").map(|s| s.as_str()),
            Some(session_key),
            &session_id,
        );

        if let Some((chat_id, custom_msg)) = self
            .session_manager
            .take_restore_notification(&session_id)
            .await
        {
            let msg = custom_msg.as_deref().unwrap_or("正在恢复会话...");
            if let Err(e) = self.send_outbound_simplified(&chat_id, channel, msg).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "failed to send restore notification via simplified outbound"
                );
            }
        }

        self.forward_to_plugin(channel, message, &session_id).await
    }

    /// Forward via explicit `session_id` (old path, backward compatible).
    async fn route_via_session_id(
        &self,
        channel: &str,
        message: &Message,
        session_id: &str,
    ) -> Result<(), GatewayError> {
        self.emit_route_debug_log(
            message.metadata.get("trace_id").map(|s| s.as_str()),
            None,
            session_id,
        );
        self.forward_to_plugin(channel, message, session_id).await
    }

    /// Route an incoming message to the appropriate agent.
    ///
    /// Supports two metadata formats for session resolution:
    /// 1. New path: `session_key` → call `SessionManager::resolve()` to get session_id
    /// 2. Old path: `session_id` → validate directly in active sessions table
    ///
    /// If both are missing, sends a user-visible error via the plugin and
    /// returns `NoRoutingKey`.
    pub async fn route_message(
        &self,
        channel: &str,
        message: Message,
        account_id: Option<&str>,
    ) -> Result<(), GatewayError> {
        if let Some(session_key) = message.metadata.get("session_key") {
            if !session_key.is_empty() {
                return self
                    .route_via_session_key(channel, &message, session_key, account_id)
                    .await;
            }
        }

        if let Some(session_id) = message.metadata.get("session_id") {
            if !session_id.is_empty() {
                return self
                    .route_via_session_id(channel, &message, session_id)
                    .await;
            }
        }

        self.send_user_error(channel, &message).await;
        Err(GatewayError::NoRoutingKey)
    }

    /// Runs the inbound processor chain on the given [`NormalizedMessage`].
    /// Falls back to raw content on registry absence or processor error.
    pub async fn process_inbound_chain(&self, normalized: &NormalizedMessage) -> ProcessedMessage {
        let extra_meta = build_extra_metadata(normalized);
        let registry = self.processor_registry.read().unwrap().clone();
        let Some(registry) = registry else {
            return ProcessedMessage {
                content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                    normalized.content.to_string(),
                )],
                metadata: extra_meta,
            };
        };

        match registry.process_inbound(normalized.clone()).await {
            Ok(mut processed) => {
                processed.metadata.extend(extra_meta);
                processed
            }
            Err(e) => {
                tracing::warn!(?e, "processor chain failed, falling back to raw content");
                ProcessedMessage {
                    content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                        normalized.content.to_string(),
                    )],
                    metadata: extra_meta,
                }
            }
        }
    }
}

/// Build extra metadata map from [`NormalizedMessage`] fields.
///
/// Propagates `thread_id`, `media_refs`, `account_id`, `chat_name`, and
/// `trace_id` so they are available downstream in the Gateway.
///
/// Note: `message_type` is injected by the Processor Chain (SessionRouter),
/// not by the Gateway — see design doc `data-flow.md`.
fn build_extra_metadata(normalized: &NormalizedMessage) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    if let Some(ref thread_id) = normalized.thread_id {
        meta.insert("thread_id".to_string(), thread_id.clone());
    }
    meta.insert(
        "media_refs".to_string(),
        serde_json::to_string(&normalized.media_refs).unwrap_or_else(|_| "[]".to_string()),
    );
    if !normalized.account_id.is_empty() {
        meta.insert("account_id".to_string(), normalized.account_id.clone());
    }
    if !normalized.chat_name.is_empty() {
        meta.insert("chat_name".to_string(), normalized.chat_name.clone());
    }
    if !normalized.trace_id.is_empty() {
        meta.insert("trace_id".to_string(), normalized.trace_id.clone());
    }
    meta
}
