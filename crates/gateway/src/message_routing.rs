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
use closeclaw_processor_chain::context::inject_chain_dispatcher_keys;
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
        let reply_ref = self.session_manager.get_reply_ref(session_id).await;
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": &message.content}}),
        };
        plugin
            .send(
                &output,
                &message.to,
                thread_id.as_deref(),
                reply_ref.as_deref(),
            )
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
            let _ = plugin.send(&err_output, &message.to, None, None).await;
        }
    }

    /// Emit a `route.decision` debug log entry for the given trace context.
    fn emit_route_debug_log(
        &self,
        trace_id: Option<&str>,
        session_key: Option<&str>,
        session_id: &str,
        parent: Option<&closeclaw_debug_log::TraceContext>,
    ) {
        if let Some(tid) = trace_id {
            let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
            debug_log_emitter::emit_debug_event(debug_log_emitter::EmitEventParams {
                ctx: debug_log_emitter::DebugLogContext::new(guard.as_ref(), tid, session_key),
                level: LogLevel::Info,
                source_module: "gateway",
                event_type: "route.decision",
                payload: serde_json::json!({
                    "session_key": session_key.unwrap_or_default(),
                    "session_id": session_id,
                }),
                parent,
            });
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
        // Design doc: Gateway resolves agent_id from bot→Agent bindings
        // before passing it to SessionManager.
        let agent_id = Self::resolve_agent_id(&self.config.bot_agent_bindings, &message.to);
        let session_id = self
            .session_manager
            .resolve(session_key, channel, message, account_id, &agent_id)
            .await
            .map_err(|e| GatewayError::AdapterError(e.to_string()))?;

        self.emit_route_debug_log(
            message.metadata.get("trace_id").map(|s| s.as_str()),
            Some(session_key),
            &session_id,
            None, // no parent context available in outbound routing path
        );

        if let Some((chat_id, custom_msg)) = self
            .session_manager
            .take_restore_notification(&session_id)
            .await
        {
            let msg = custom_msg
                .as_deref()
                .unwrap_or(closeclaw_session::notifications::RESTORE_NOTIFICATION_DEFAULT_TEXT);
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
            None, // no parent context available in outbound routing path
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
            // No registry — bypass path. Chain dispatcher keys are still
            // required for metadata contract consistency.
            return ProcessedMessage {
                content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                    normalized.content.to_string(),
                )],
                metadata: fallback_metadata(extra_meta, normalized),
            };
        };

        match registry.process_inbound(normalized.clone()).await {
            Ok(mut processed) => {
                processed.metadata.extend(extra_meta);
                processed
            }
            Err(e) => {
                tracing::warn!(?e, "processor chain failed, falling back to raw content");
                // Fallback on error — inject chain dispatcher keys
                // for metadata contract consistency.
                ProcessedMessage {
                    content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                        normalized.content.to_string(),
                    )],
                    metadata: fallback_metadata(extra_meta, normalized),
                }
            }
        }
    }
}

/// Build a metadata map for fallback branches (no-registry or chain error).
///
/// Copies `extra_meta` (Gateway-owned keys: thread_id, media_refs, etc.)
/// and injects chain dispatcher keys (message_type, unavailable_media) so
/// the metadata contract is consistent regardless of processing path.
fn fallback_metadata(
    extra_meta: HashMap<String, String>,
    normalized: &NormalizedMessage,
) -> HashMap<String, String> {
    let mut meta = extra_meta;
    inject_chain_dispatcher_keys(
        &mut meta,
        &normalized.message_type,
        &normalized.unavailable_media,
    );
    meta
}

/// Build extra metadata map from [`NormalizedMessage`] fields.
///
/// Propagates `thread_id`, `media_refs`, `account_id`, `chat_name`,
/// and `trace_id` so they are available downstream in the Gateway.
///
/// Note: `message_type` and `unavailable_media` are injected by the
/// Processor Chain (chain dispatcher in `MessageContext::from_normalized`
/// or fallback branches in `process_inbound_chain`), not by the Gateway
/// — see design doc `inbound-flow.md`.
fn build_extra_metadata(normalized: &NormalizedMessage) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    if let Some(ref thread_id) = normalized.thread_id {
        meta.insert("thread_id".to_string(), thread_id.clone());
    }
    if let Some(ref reply_ref) = normalized.reply_ref {
        meta.insert("reply_ref".to_string(), reply_ref.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::im_plugin::{MediaRef, MediaType, MessageType};

    fn make_normalized(overrides: impl FnOnce(&mut NormalizedMessage)) -> NormalizedMessage {
        let mut msg = NormalizedMessage {
            platform: "test".into(),
            sender_id: "sender1".into(),
            peer_id: "peer1".into(),
            content: "hello".into(),
            message_type: MessageType::Text,
            media_refs: vec![],
            unavailable_media: vec![],
            timestamp: 0,
            ..Default::default()
        };
        overrides(&mut msg);
        msg
    }

    #[test]
    fn media_refs_empty_produces_empty_json_array() {
        let normalized = make_normalized(|_| {});
        let meta = build_extra_metadata(&normalized);
        assert_eq!(meta.get("media_refs").unwrap(), "[]");
    }

    #[test]
    fn media_refs_non_empty_serializes_correctly() {
        let normalized = make_normalized(|n| {
            n.media_refs = vec![MediaRef {
                key: "ref_a".into(),
                path: "/media/a".into(),
                media_type: MediaType::Image,
                size: 100,
                mime: "image/png".into(),
            }];
        });
        let meta = build_extra_metadata(&normalized);
        let parsed: Vec<MediaRef> = serde_json::from_str(meta.get("media_refs").unwrap()).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].key, "ref_a");
        assert_eq!(parsed[0].media_type, MediaType::Image);
    }

    #[test]
    fn thread_id_propagated_when_present() {
        let normalized = make_normalized(|n| {
            n.thread_id = Some("t_123".into());
        });
        let meta = build_extra_metadata(&normalized);
        assert_eq!(meta.get("thread_id").unwrap(), "t_123");
    }

    #[test]
    fn thread_id_absent_when_none() {
        let normalized = make_normalized(|_| {});
        let meta = build_extra_metadata(&normalized);
        assert!(!meta.contains_key("thread_id"));
    }

    #[test]
    fn account_id_propagated_when_non_empty() {
        let normalized = make_normalized(|n| {
            n.account_id = "acc_1".into();
        });
        let meta = build_extra_metadata(&normalized);
        assert_eq!(meta.get("account_id").unwrap(), "acc_1");
    }

    #[test]
    fn account_id_absent_when_empty() {
        let normalized = make_normalized(|_| {});
        let meta = build_extra_metadata(&normalized);
        assert!(!meta.contains_key("account_id"));
    }

    #[test]
    fn chat_name_propagated_when_non_empty() {
        let normalized = make_normalized(|n| {
            n.chat_name = "General".into();
        });
        let meta = build_extra_metadata(&normalized);
        assert_eq!(meta.get("chat_name").unwrap(), "General");
    }

    #[test]
    fn trace_id_propagated_when_non_empty() {
        let normalized = make_normalized(|n| {
            n.trace_id = "tr_abc".into();
        });
        let meta = build_extra_metadata(&normalized);
        assert_eq!(meta.get("trace_id").unwrap(), "tr_abc");
    }

    #[test]
    fn all_fields_combined() {
        let normalized = make_normalized(|n| {
            n.thread_id = Some("t_1".into());
            n.reply_ref = Some("r_1".into());
            n.media_refs = vec![MediaRef {
                key: "r1".into(),
                path: "/media/r1".into(),
                media_type: MediaType::Image,
                size: 500,
                mime: "image/jpeg".into(),
            }];
            n.unavailable_media = vec!["u1".into(), "u2".into()];
            n.account_id = "a1".into();
            n.chat_name = "chat".into();
            n.trace_id = "tr1".into();
        });
        // build_extra_metadata propagates thread_id, reply_ref, media_refs,
        // account_id, chat_name, trace_id — but NOT unavailable_media
        // (which is injected by the chain dispatcher).
        let meta = build_extra_metadata(&normalized);
        assert_eq!(meta.len(), 6);
        assert_eq!(meta.get("thread_id").unwrap(), "t_1");
        assert_eq!(meta.get("reply_ref").unwrap(), "r_1");
        assert!(!meta.contains_key("unavailable_media"));
        assert_eq!(meta.get("account_id").unwrap(), "a1");
        assert_eq!(meta.get("chat_name").unwrap(), "chat");
        assert_eq!(meta.get("trace_id").unwrap(), "tr1");
    }

    #[test]
    fn build_extra_metadata_no_unavailable_media() {
        // After Step 1.2: build_extra_metadata must NOT include unavailable_media
        // (chain dispatcher is responsible for injecting it).
        let normalized = make_normalized(|n| {
            n.unavailable_media = vec!["img_x".into()];
        });
        let meta = build_extra_metadata(&normalized);
        assert!(!meta.contains_key("unavailable_media"),
            "build_extra_metadata must not include unavailable_media; chain dispatcher is responsible for this key");
    }
}
