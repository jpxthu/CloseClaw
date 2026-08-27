//! Auto-compaction logic for `SessionMessageHandler`.
//!
//! Extracted from `session_handler.rs` to keep impl blocks under the
//! 100-line project limit. This module hosts the compaction pipeline:
//! token estimation, warning state handling, auto-compact
//! execution, and circuit-breaker notification.

use super::session_handler::SessionMessageHandler;
use crate::session_manager::compact::{load_compact_inputs, PreloadedCompactInputs};
use crate::OutputTx;
use closeclaw_common::RunningStats;
#[allow(deprecated)]
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::Message as ChatMessage;
use closeclaw_llm::ProviderModelKnowledge;
use closeclaw_session::compaction::{CompactionMessage, TokenWarningState};
use std::sync::Arc;

// ── Compaction: token estimation + state handling ──
impl SessionMessageHandler {
    /// Handle the Warning token state: emit a one-time user notification.
    async fn handle_token_warning_state(&self, session_id: &str, tokens: usize, model: &str) {
        tracing::warn!(
            session_id,
            tokens,
            model = %model,
            "token warning: approaching context limit"
        );
        {
            let mut warned = self.has_warned.lock().expect("has_warned poisoned");
            if *warned {
                return;
            }
            *warned = true;
        }
        send_output(&self.output_tx, "⚠️ 对话即将压缩，可输入 /compact 手动管理").await;
    }

    /// Truncate persistent transcript to `max_history_messages`.
    async fn truncate_before_compact(&self, session_id: &str) {
        let max = {
            let svc = self.compaction_service.lock().await;
            svc.config().max_history_messages
        };
        if let Some(max) = max {
            if let Some(cs) = self
                .session_manager
                .get_conversation_session(session_id)
                .await
            {
                let dropped = { cs.write().await.truncate_transcript_to_limit(Some(max)) };
                if dropped > 0 {
                    tracing::info!(session_id, max, dropped, "历史截断（消息上限截断）");
                }
            }
        }
    }

    /// Check token usage and trigger auto-compaction if needed.
    pub(super) async fn check_and_run_auto_compact(&self, session_id: &str) {
        // Step 1: truncate persistent transcript before loading inputs.
        self.truncate_before_compact(session_id).await;
        // Step 2: load inputs from (now-truncated) persistent history.
        let Some((model, llm_messages, stats)) =
            load_compact_inputs(&self.session_manager, session_id).await
        else {
            return;
        };
        // Step 3: estimate tokens and act on warning state.
        let (_compaction_msgs, warning, tokens) = self
            .estimate_and_check_state(&llm_messages, &model, &stats)
            .await;
        match warning {
            TokenWarningState::Normal => {
                *self.has_warned.lock().expect("has_warned poisoned") = false;
            }
            TokenWarningState::Warning => {
                self.handle_token_warning_state(session_id, tokens, &model)
                    .await;
            }
            TokenWarningState::AutoCompactTriggered => {
                let preloaded = PreloadedCompactInputs {
                    model,
                    llm_messages,
                    stats,
                };
                self.run_auto_compact(session_id, preloaded).await;
            }
        }
    }
}

// ── Compaction: circuit breaker + execution ──
impl SessionMessageHandler {
    /// Estimate tokens and determine the warning state for the current conversation.
    async fn estimate_and_check_state(
        &self,
        llm_messages: &[ChatMessage],
        model: &str,
        stats: &RunningStats,
    ) -> (Vec<CompactionMessage>, TokenWarningState, usize) {
        let compaction_msgs: Vec<CompactionMessage> = llm_messages
            .iter()
            .map(|m| CompactionMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();
        let cpt = {
            let svc = self.compaction_service.lock().await;
            svc.config().chars_per_token
        };
        let tokens =
            closeclaw_session::compaction::estimate_total_tokens(stats, &compaction_msgs, cpt);
        let kb_window = self
            .model_knowledge
            .as_ref()
            .and_then(|kb| find_context_window_for_model(kb, model));
        let warning = {
            let svc = self.compaction_service.lock().await;
            svc.token_warning_state(tokens, model, kb_window)
        };
        (compaction_msgs, warning, tokens)
    }

    /// Inject a one-time assistant message when the circuit breaker trips.
    async fn inject_circuit_breaker_notification(&self, session_id: &str) {
        let should_notify = {
            let mut flag = self.has_circuit_break_notified.lock().expect("poisoned");
            if *flag {
                return;
            }
            *flag = true;
            true
        };
        let msg = "自动压缩已暂停，建议手动 /compact";
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            cs.write()
                .await
                .append_transcript("assistant", vec![ContentBlock::Text(msg.to_string())]);
        }
        let _ = should_notify;
    }

    /// Execute auto-compaction: check breaker, snapshot, compact, finalize.
    async fn run_auto_compact(&self, session_id: &str, preloaded: PreloadedCompactInputs) {
        {
            let breaker = self.compaction_service.lock().await;
            if breaker.consecutive_failures() >= breaker.config().max_consecutive_failures {
                self.inject_circuit_breaker_notification(session_id).await;
                return;
            }
        }
        // Build ChatFn: pure LLM forwarding layer.
        let fc = Arc::clone(&self.fallback_client);
        let chat_fn = build_chat_fn(fc);
        // Lock CompactionService and call SessionManager::compact.
        // SessionManager::compact handles apply/rollback internally.
        let mut svc = self.compaction_service.lock().await;
        let result = self
            .session_manager
            .compact(session_id, None, true, &mut svc, &chat_fn, Some(preloaded))
            .await;
        drop(svc);
        match result {
            Ok(r) => {
                tracing::info!(
                    session_id,
                    before = r.before_char_count,
                    after = r.after_char_count,
                    "auto compact completed"
                );
                // Reset circuit-breaker notification flag on success.
                *self
                    .has_circuit_break_notified
                    .lock()
                    .expect("has_circuit_break_notified poisoned") = false;
            }
            Err(e) => {
                tracing::warn!(session_id, error = %e, "auto compact failed");
                self.compaction_service.lock().await.record_failure();
            }
        }
    }
}

// ── Compaction helpers ──

/// Look up a model's context window from the knowledge base.
pub(crate) fn find_context_window_for_model(
    knowledge: &ProviderModelKnowledge,
    model: &str,
) -> Option<u32> {
    const PROVIDERS: &[&str] = &["minimax", "glm", "volcengine", "deepseek", "mimo"];
    for provider in PROVIDERS {
        if let Some(params) = knowledge.find(provider, model) {
            return Some(params.context_window);
        }
    }
    None
}

/// Build a [`ChatFn`] that forwards messages directly to the LLM client.
#[allow(deprecated)]
pub(crate) fn build_chat_fn(fc: Arc<FallbackClient>) -> closeclaw_session::compaction::ChatFn {
    Arc::new(move |model, messages| {
        let fc = Arc::clone(&fc);
        Box::pin(async move {
            use closeclaw_llm::{ChatRequest, Message as LlmMessage};

            let llm_messages: Vec<LlmMessage> = messages
                .iter()
                .map(|m| LlmMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                })
                .collect();
            let request = ChatRequest {
                model,
                messages: llm_messages,
                temperature: 0.0,
                max_tokens: Some(4096),
            };
            let (response, retries) = fc.chat(request).await.map_err(|e| e.to_string())?;
            Ok((response.content, retries))
        })
    })
}

pub(crate) async fn send_output(output_tx: &OutputTx, text: &str) {
    let guard = output_tx.read().await;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send((text.to_string(), vec![])).await;
    }
}
