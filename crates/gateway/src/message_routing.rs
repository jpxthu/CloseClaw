//! Message routing and inbound chain processing for the Gateway.
//!
//! Extracted from `lib.rs` to keep the main file under the 1000-line limit.

use super::Gateway;
use crate::debug_log_emitter;
use crate::types::{GatewayError, InboundChainInput, Message};
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::processor::ProcessedMessage;
use closeclaw_debug_log::LogLevel;
use std::collections::HashMap;

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
                            "\u{4F1A}\u{8BDD}\u{8DEF}\u{7531}\u{5931}\u{8D25}\u{FF0C}\u{8BF7}\u{91CD}\u{8BD5}"
                    }
                }),
            };
            let _ = plugin.send(&err_output, &message.to, None).await;
        }
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
        // --- New path: session_key → SessionManager::resolve() ---
        if let Some(session_key) = message.metadata.get("session_key") {
            if !session_key.is_empty() {
                let session_id = self
                    .session_manager
                    .resolve(session_key, channel, &message, account_id)
                    .await
                    .map_err(|e| GatewayError::AdapterError(e.to_string()))?;
                // Debug log: route.decision (new path)
                if let Some(trace_id) = message.metadata.get("trace_id") {
                    let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
                    debug_log_emitter::emit_debug_event(
                        guard.as_ref(),
                        trace_id,
                        Some(session_key),
                        LogLevel::Info,
                        "gateway",
                        "route.decision",
                        serde_json::json!({
                            "session_key": session_key,
                            "session_id": session_id,
                        }),
                    );
                }
                // Send restore notification through outbound chain (if any).
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
                return self.forward_to_plugin(channel, &message, &session_id).await;
            }
        }

        // --- Fallback: session_id (old path, backward compatible) ---
        if let Some(session_id) = message.metadata.get("session_id") {
            if !session_id.is_empty() {
                // Debug log: route.decision (fallback path)
                if let Some(trace_id) = message.metadata.get("trace_id") {
                    let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
                    debug_log_emitter::emit_debug_event(
                        guard.as_ref(),
                        trace_id,
                        None,
                        LogLevel::Info,
                        "gateway",
                        "route.decision",
                        serde_json::json!({
                            "session_id": session_id,
                        }),
                    );
                }
                return self.forward_to_plugin(channel, &message, session_id).await;
            }
        }

        // --- No key fallback: both missing/empty ---
        self.send_user_error(channel, &message).await;
        Err(GatewayError::NoRoutingKey)
    }

    /// Runs the inbound processor chain on a [`NormalizedMessage`] built from `input`.
    /// Falls back to raw content on registry absence or processor error.
    pub async fn process_inbound_chain(&self, input: &InboundChainInput) -> ProcessedMessage {
        let extra_meta = build_extra_metadata(input);
        let registry = self.processor_registry.read().unwrap().clone();
        let Some(registry) = registry else {
            return ProcessedMessage {
                content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                    input.content.to_string(),
                )],
                metadata: extra_meta,
            };
        };

        let normalized = closeclaw_common::im_plugin::NormalizedMessage {
            platform: input.platform.to_string(),
            sender_id: input.sender_id.to_string(),
            peer_id: input.peer_id.to_string(),
            content: input.content.to_string(),
            timestamp: input.timestamp_ms,
            message_type: input.message_type.clone(),
            media_refs: input.media_refs.clone(),
            thread_id: input.thread_id.clone(),
            account_id: input.account_id.clone().unwrap_or_default(),
        };

        match registry.process_inbound(normalized).await {
            Ok(mut processed) => {
                processed.metadata.extend(extra_meta);
                processed
            }
            Err(e) => {
                tracing::warn!(?e, "processor chain failed, falling back to raw content");
                ProcessedMessage {
                    content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                        input.content.to_string(),
                    )],
                    metadata: extra_meta,
                }
            }
        }
    }
}

/// Build extra metadata map from inbound chain input fields.
///
/// Propagates `thread_id`, `message_type`, and `media_refs`
/// so they are available downstream in the Gateway.
fn build_extra_metadata(input: &InboundChainInput) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    if let Some(ref thread_id) = input.thread_id {
        meta.insert("thread_id".to_string(), thread_id.clone());
    }
    meta.insert(
        "message_type".to_string(),
        serde_json::to_string(&input.message_type).unwrap_or_else(|_| "text".to_string()),
    );
    meta.insert(
        "media_refs".to_string(),
        serde_json::to_string(&input.media_refs).unwrap_or_else(|_| "[]".to_string()),
    );
    if let Some(ref account_id) = input.account_id {
        meta.insert("account_id".to_string(), account_id.clone());
    }
    if let Some(ref chat_name) = input.chat_name {
        meta.insert("chat_name".to_string(), chat_name.clone());
    }
    if let Some(ref trace_id) = input.trace_id {
        meta.insert("trace_id".to_string(), trace_id.clone());
    }
    meta
}
