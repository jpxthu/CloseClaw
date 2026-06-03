//! Streaming LLM call implementation for SessionMessageHandler.
//!
//! Extracted from `session_handler.rs` to keep file sizes under the
//! 500-line project limit.

use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::session_handler::{MessageMetadata, SessionMessageHandler};
use crate::gateway::session_manager::SessionManager;
use crate::gateway::system_prompt_inject::{build_dynamic_sections, build_full_system_prompt};
use crate::llm::client::UnifiedChatClient;
use crate::llm::session::ChatSession;
use crate::llm::session_state::LlmState;
use crate::llm::streaming::{StreamDone, StreamingSink};
use crate::llm::types::{ContentBlock, ContentDelta, StreamEvent, UnifiedResponse, UnifiedUsage};
use crate::llm::{LLMError, Message as ChatMessage};

impl SessionMessageHandler {
    /// Make a streaming LLM call using UnifiedChatClient.
    pub(super) async fn call_llm_streaming(
        unified_client: &Arc<UnifiedChatClient>,
        content: &str,
        meta: &MessageMetadata,
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) -> Result<UnifiedResponse, LLMError> {
        let (static_prompt_opt, turn_count, workdir_path) =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs_read = cs.read().await;
                (
                    cs_read.system_prompt().map(|s| s.to_string()),
                    cs_read.turn_count(),
                    cs_read.workdir().to_string_lossy().into_owned(),
                )
            } else {
                (None, 0, String::new())
            };

        let dynamic_sections =
            build_dynamic_sections(turn_count, meta, Some(workdir_path.as_str()));
        let full_prompt = build_full_system_prompt(static_prompt_opt.as_deref(), &dynamic_sections);

        let mut messages = vec![];
        if !full_prompt.is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: full_prompt,
            });
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
        });

        let internal_request = crate::llm::types::InternalRequest {
            model: String::new(),
            messages: messages
                .iter()
                .map(|m| crate::llm::types::InternalMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                })
                .collect(),
            temperature: 0.7,
            max_tokens: None,
            stream: true,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: crate::session::persistence::ReasoningLevel::default(),
        };

        // Get streaming sink from session
        let sink: Option<Arc<dyn StreamingSink>> =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs_read = cs.read().await;
                cs_read.streaming_sink().cloned()
            } else {
                None
            };

        // Set LLM state to Requesting before dispatching the stream.
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            cs.read().await.set_llm_state(LlmState::Requesting);
        }

        let mut stream = match unified_client.chat_streaming(internal_request).await {
            Ok(s) => s,
            Err(e) => {
                let msg = e.to_string();
                tracing::error!(error = %msg, "streaming LLM call failed");
                if let Some(ref s) = sink {
                    s.send_error(msg.clone());
                }
                // Reset LLM state on error.
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    cs.read().await.set_llm_state(LlmState::Idle);
                }
                return Err(LLMError::ApiError(msg));
            }
        };

        let mut full_text = String::new();
        let mut last_usage = None;
        let mut first_token = true;

        // Acquire this session's cancellation token so a streaming
        // request can be aborted mid-stream by a cascade stop. Fall
        // back to a never-cancelled token if the session is gone.
        let cancel_token: CancellationToken =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read().await.cancel_token().clone()
            } else {
                CancellationToken::new()
            };

        // Loop body: race the next stream event against cancellation.
        // We need an explicit `loop { tokio::select! { ... } }` because
        // a plain `while let Some(...) = stream.next().await` would
        // block on the stream and ignore the cancel token.
        loop {
            let event_result = tokio::select! {
                next = stream.next() => next,
                _ = cancel_token.cancelled() => {
                    // Restore idle state so the session can accept the
                    // next request. Tool/child cleanup is owned by
                    // `stop()` itself; we just abort the in-flight LLM
                    // stream here.
                    if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                        cs.read().await.set_llm_state(LlmState::Idle);
                    }
                    if let Some(ref s) = sink {
                        s.send_error("cancelled".to_string());
                    }
                    tracing::info!(session_id = %session_id, "streaming LLM request cancelled");
                    return Err(LLMError::Cancelled);
                }
            };
            let Some(event_result) = event_result else {
                // Stream ended normally.
                break;
            };
            match event_result {
                Ok(StreamEvent::BlockDelta {
                    delta: ContentDelta::Text { text },
                    ..
                }) => {
                    // Transition to Receiving on first text delta.
                    if first_token {
                        if let Some(cs) = session_manager.get_conversation_session(session_id).await
                        {
                            cs.read().await.set_llm_state(LlmState::Receiving);
                        }
                        first_token = false;
                    }
                    full_text.push_str(&text);
                    if let Some(ref s) = sink {
                        s.send_text(&text);
                    }
                }
                Ok(StreamEvent::MessageEnd { usage, .. }) => {
                    last_usage = usage;
                    if let Some(ref s) = sink {
                        s.send_done(StreamDone {
                            model: String::new(),
                            usage: last_usage.clone(),
                        });
                    }
                    break;
                }
                Ok(StreamEvent::Error { message }) => {
                    tracing::error!(error = %message, "streaming LLM call failed");
                    if let Some(ref s) = sink {
                        s.send_error(message.clone());
                    }
                    return Err(LLMError::ApiError(message));
                }
                Err(e) => {
                    let msg = e.to_string();
                    tracing::error!(error = %msg, "streaming LLM call failed");
                    if let Some(ref s) = sink {
                        s.send_error(msg.clone());
                    }
                    return Err(LLMError::ApiError(msg));
                }
                _ => {}
            }
        }

        // Reset LLM state to Idle after stream completes.
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            cs.read().await.set_llm_state(LlmState::Idle);
        }

        Ok(UnifiedResponse {
            content_blocks: if full_text.is_empty() {
                vec![]
            } else {
                vec![ContentBlock::Text(full_text)]
            },
            usage: last_usage.unwrap_or(UnifiedUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            finish_reason: None,
        })
    }
}
