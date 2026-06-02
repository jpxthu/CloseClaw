//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses to IM adapters.

use super::{Gateway, GatewayError, Message};

impl Gateway {
    /// Send an outbound message (agent response) through the processor chain.
    ///
    /// 1. Resolve `chat_id` from `session_id` via `SessionManager::get_chat_id`.
    /// 2. If `renderer` is present → run processor chain, deserialize
    ///    `dsl_result` from `processed.metadata["dsl_result"]`, call
    ///    `renderer.render(clean_content, dsl_result)`, dispatch by `msg_type`.
    /// 3. If `renderer` is absent but `processor_registry` is present → inspect
    ///    `msg_type` from processed content JSON (existing behavior).
    /// 4. If neither is present → send `raw_output` as plain text (bypass).
    /// 5. If `suppress == true` → return `Ok` without sending.
    /// 6. After each successful send, trigger checkpoint persistence.
    pub async fn send_outbound(
        &self,
        session_id: &str,
        channel: &str,
        raw_output: &str,
        content_blocks: Vec<crate::llm::types::ContentBlock>,
    ) -> Result<(), GatewayError> {
        // Step 1: resolve chat_id
        let chat_id = self
            .session_manager
            .get_chat_id(session_id)
            .await
            .ok_or(GatewayError::MissingSessionId)?;

        // Step 2: resolve adapter
        let adapter = {
            let adapters = self.adapters.read().await;
            adapters
                .get(channel)
                .ok_or_else(|| GatewayError::UnknownChannel(channel.to_string()))?
                .clone()
        };

        // Step 3: bypass, renderer path, or processor-chain path
        let processed_content = if let Some(ref renderer) = self.renderer {
            let registry = self.processor_registry.as_ref().ok_or_else(|| {
                GatewayError::OutboundError("renderer requires processor registry".into())
            })?;
            let processed = crate::processor_chain::ProcessedMessage {
                content: raw_output.to_string(),
                metadata: serde_json::Map::new(),
                suppress: false,
                content_blocks: content_blocks.clone(),
            };
            let result = registry
                .process_outbound(processed)
                .await
                .map_err(|e| GatewayError::OutboundError(e.to_string()))?;
            if result.suppress {
                return Ok(());
            }

            // Deserialize dsl_result from metadata
            let dsl_result: Option<crate::processor_chain::DslParseResult> = result
                .metadata
                .get("dsl_result")
                .and_then(|v| v.as_str())
                .and_then(|s| serde_json::from_str(s).ok());

            // Step 1.2: pass the full ContentBlock stream to the renderer.
            // The renderer decides per-block rendering (text/thinking/tool_use/tool_result)
            // and extracts DSL-stripped plain text internally for the single-Text
            // fast path. Previously this call site derived `clean_content` from
            // `dsl_result` itself; that responsibility now lives in the renderer.
            //
            // Fallback: when content_blocks is empty (e.g. legacy caller that
            // didn't produce structured blocks), wrap the raw `result.content`
            // in a single Text block so simple text paths still work.
            let owned_fallback;
            let blocks_for_render: &[crate::llm::types::ContentBlock] =
                if result.content_blocks.is_empty() {
                    owned_fallback = vec![crate::llm::types::ContentBlock::Text(
                        result.content.clone(),
                    )];
                    &owned_fallback
                } else {
                    &result.content_blocks
                };

            // Preserve a plain-text fallback string in case the renderer's
            // text branch returns a payload missing the `content.text` field
            // (defensive: older code paths may rely on it).
            let content = dsl_result
                .as_ref()
                .map(|r| r.clean_content.as_str())
                .unwrap_or(&result.content);

            let rendered = renderer.render(blocks_for_render, dsl_result.as_ref());
            let payload_str =
                serde_json::to_string(&rendered.payload).unwrap_or_else(|_| "{}".to_string());

            match rendered.msg_type.as_str() {
                "text" => {
                    let text = rendered
                        .payload
                        .get("content")
                        .and_then(|v| v.get("text"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(content)
                        .to_string();
                    let msg = Message {
                        id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
                        from: "agent".to_string(),
                        to: chat_id.clone(),
                        content: text,
                        channel: channel.to_string(),
                        timestamp: chrono::Utc::now().timestamp(),
                        metadata: std::collections::HashMap::new(),
                    };
                    adapter.send_message(&msg).await?;
                    self.persist_outbound_checkpoint(session_id, &msg).await;
                    return Ok(());
                }
                "interactive" => {
                    adapter.send_card_json(&chat_id, &payload_str).await?;
                    let msg = Message {
                        id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
                        from: "agent".to_string(),
                        to: chat_id.clone(),
                        content: payload_str.clone(),
                        channel: channel.to_string(),
                        timestamp: chrono::Utc::now().timestamp(),
                        metadata: std::collections::HashMap::new(),
                    };
                    self.persist_outbound_checkpoint(session_id, &msg).await;
                    return Ok(());
                }
                _ => {
                    return Err(GatewayError::OutboundError(format!(
                        "unknown msg_type: {}",
                        rendered.msg_type
                    )))
                }
            }
        } else if let Some(ref registry) = self.processor_registry {
            let processed = crate::processor_chain::ProcessedMessage {
                content: raw_output.to_string(),
                metadata: serde_json::Map::new(),
                suppress: false,
                content_blocks: content_blocks.clone(),
            };
            let result = registry.process_outbound(processed).await;
            match result {
                Ok(p) => {
                    if p.suppress {
                        return Ok(());
                    }
                    p.content
                }
                Err(e) => return Err(GatewayError::OutboundError(e.to_string())),
            }
        } else {
            raw_output.to_string()
        };

        // Step 4: inspect msg_type and dispatch
        if let Ok(json) =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&processed_content)
        {
            if let Some(msg_type) = json.get("msg_type") {
                match msg_type.as_str().unwrap_or("") {
                    "text" => {
                        let msg = Message {
                            id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
                            from: "agent".to_string(),
                            to: chat_id.clone(),
                            content: json
                                .get("content")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&processed_content)
                                .to_string(),
                            channel: channel.to_string(),
                            timestamp: chrono::Utc::now().timestamp(),
                            metadata: std::collections::HashMap::new(),
                        };
                        adapter.send_message(&msg).await?;
                        self.persist_outbound_checkpoint(session_id, &msg).await;
                        return Ok(());
                    }
                    "interactive" => {
                        let card_json = json
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&processed_content)
                            .to_string();
                        adapter.send_card_json(&chat_id, &card_json).await?;
                        let msg = Message {
                            id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
                            from: "agent".to_string(),
                            to: chat_id.clone(),
                            content: card_json,
                            channel: channel.to_string(),
                            timestamp: chrono::Utc::now().timestamp(),
                            metadata: std::collections::HashMap::new(),
                        };
                        self.persist_outbound_checkpoint(session_id, &msg).await;
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }

        // Fallback: treat as plain text
        let msg = Message {
            id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
            from: "agent".to_string(),
            to: chat_id,
            content: processed_content,
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
        };
        adapter.send_message(&msg).await?;
        self.persist_outbound_checkpoint(session_id, &msg).await;
        Ok(())
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
        cp.touch();
        if let Err(e) = cm.save(cp).await {
            tracing::warn!(session_id, "failed to save checkpoint: {}", e);
        }
    }
}
