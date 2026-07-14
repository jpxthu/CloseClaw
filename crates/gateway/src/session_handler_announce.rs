//! Announce integration and LLM-finishing helpers for SessionMessageHandler.
//!
//! Extracted from `session_handler.rs` to keep the file under the
//! 500-line project limit. This module hosts two closely related
//! concerns that share the same call sites:
//!
//! 1. **Announce integration** (Step 1.5) — `maybe_push_announce` and
//!    `drain_announce_events` wrap the two `SessionManager` methods
//!    that let run-mode child sessions notify their parent and let
//!    parents drain queued announces before processing the next
//!    pending user message.
//! 2. **LLM finishing** — `finish_llm`, `clear_busy_and_send`, and
//!    `drain_pending_loop` are the post-LLM completion pipeline that
//!    clears the busy flag, surfaces the response, pushes the
//!    announce, and processes any queued pending messages. They were
//!    co-located with the announce calls and grew large enough that
//!    moving them together was the natural fix for the line-count
//!    constraint.

use std::sync::Arc;

use super::session_handler::SessionMessageHandler;
use super::OutputTx;
use crate::outbound::StreamResult;
use crate::session_manager::SessionManager;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::run_health::RecoverableAction;
use closeclaw_tasks::NotificationPriority;
use tokio::time::Instant;

/// Turn-level timing metadata passed through the health
/// check pipeline so hard rules receive actual runtime values.
pub(super) struct TurnMetrics {
    pub turn_duration_ms: u64,
}

impl SessionMessageHandler {
    /// Clear busy flag, send output, and drain pending messages.
    ///
    /// Accepts a [`StreamResult`] (returned by the streaming LLM call) or an
    /// `LLMError`. The non-streaming `call_llm` path converts its
    /// `UnifiedResponse` into a `StreamResult` via [`StreamResult::from`]
    /// before calling this entry point.
    pub(super) async fn finish_llm(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<StreamResult, closeclaw_llm::LLMError>,
        turn_start: Instant,
        output_tx: &OutputTx,
        metrics_emitter: &Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
    ) {
        let turn_metrics = TurnMetrics {
            turn_duration_ms: turn_start.elapsed().as_millis() as u64,
        };
        let skip_drain = Self::clear_busy_and_send(
            session_manager,
            session_id,
            result,
            turn_metrics,
            output_tx,
            metrics_emitter,
        )
        .await;

        // Step 1.5: Skip drain if recovery action or session yielding.
        if skip_drain {
            tracing::info!(
                session_id = %session_id,
                "finish_llm: recovery action requested stop, skipping pending drain"
            );
            return;
        }

        // Check if session is yielding (sessions_yield called).
        // If yielding, skip draining pending messages — the turn ends here.
        // Pending messages will be processed after the session resumes.
        if Self::is_session_yielding(session_manager, session_id).await {
            tracing::info!(
                session_id = %session_id,
                "finish_llm: session is yielding, skipping pending drain"
            );
            return;
        }

        Self::drain_pending_loop(session_manager, session_id, output_tx, metrics_emitter).await;

        // NOTE: Decrement is handled by the caller (spawned task in
        // `session_handler_dispatch.rs`), NOT here. This avoids a
        // double-decrement when both `finish_llm` and the spawned task
        // call `decrement_busy()`.

        // NOTE: Cascade-termination of child sessions is NOT done here.
        // `finish_llm` is called after every LLM turn — cascading here
        // would prematurely kill session-mode children that are designed
        // to survive across turns. Cascade kill is handled by:
        // - The sweeper (idle→archive path) for normal parent session end
        // - `sessions_kill` tool for explicit parent-initiated kills
        // - `ArchiveSweeper::cascade_archive_impl` for timeout cleanup
        // See design-doc §生命周期联动 for the two correct trigger points.
    }

    /// Check if a session is in active Waiting (yielding) state.
    ///
    /// Called by [`finish_llm`] to skip draining pending messages when
    /// the session has entered yielding via `sessions_yield`.
    async fn is_session_yielding(session_manager: &Arc<SessionManager>, session_id: &str) -> bool {
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            cs.read().await.is_waiting()
        } else {
            false
        }
    }

    /// Returns `true` if the caller should skip `drain_pending_loop`
    /// (recovery action requested a stop).
    async fn clear_busy_and_send(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<StreamResult, closeclaw_llm::LLMError>,
        turn_metrics: TurnMetrics,
        output_tx: &OutputTx,
        metrics_emitter: &Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
    ) -> bool {
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            let cs = cs.write().await;
            cs.set_llm_busy(false);
            cs.set_llm_state(LlmState::Idle);
        }
        let mut skip_drain = false;
        match result {
            Ok(stream_result) => {
                // Append response to session message history. `append_response`
                // takes a `UnifiedResponse`; convert via the existing
                // `From<StreamResult> for UnifiedResponse` impl.
                let unified: closeclaw_llm::types::UnifiedResponse = stream_result.clone().into();
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    let mut cs_write = cs.write().await;
                    cs_write.append_response(unified);
                    // Cache break detection (must run before accumulate_usage
                    // so that last_cache_read_tokens still holds the previous value).
                    if let Some(info) =
                        cs_write.detect_cache_break_for_usage(stream_result.usage.cache_read_tokens)
                    {
                        tracing::warn!(
                            session_id,
                            previous = info.previous_cache_read,
                            current = info.current_cache_read,
                            drop = info.drop_tokens,
                            ratio = info.drop_ratio,
                            "Cache break detected"
                        );
                        if let Some(emitter) = metrics_emitter {
                            emitter.emit_cache_break(&info);
                        }
                    }
                    cs_write.accumulate_usage(&stream_result.usage);

                    // Run health check at turn boundary.
                    let mut recovery_action = None;
                    if let Some(checker_arc) = cs_write.health_checker() {
                        let input = crate::health_check_builders::build_health_check_input(
                            &stream_result,
                            turn_metrics.turn_duration_ms,
                        );
                        let recent_calls = cs_write.recent_tool_calls(5);
                        let hook_ctx = crate::health_check_builders::build_hook_context(
                            &stream_result,
                            recent_calls,
                        );
                        let mut checker = checker_arc.lock().await;
                        let verdict = checker.check_turn(&input, Some(&hook_ctx)).await;
                        if verdict.status != closeclaw_session::run_health::HealthStatus::Healthy {
                            tracing::warn!(
                                session_id,
                                status = ?verdict.status,
                                action = ?verdict.action,
                                "health check: unhealthy turn detected"
                            );
                            recovery_action = verdict.action;
                        }
                    }
                    drop(cs_write);

                    // Handle recovery actions from health check.
                    if let Some(action) = recovery_action {
                        skip_drain = Self::handle_recovery_action(session_id, action, output_tx);
                    }
                }
                let text = stream_result
                    .content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let guard = output_tx.read().await;
                if let Some(tx) = guard.as_ref() {
                    let _ = tx.send((text, stream_result.content_blocks)).await;
                }
            }
            Err(err) => {
                tracing::warn!(session_id, error = %err, "LLM call failed");
            }
        }
        // Step 1.5: best-effort announce to parent (run-mode child).
        Self::maybe_push_announce(session_manager, session_id).await;
        skip_drain
    }

    async fn drain_pending_loop(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        output_tx: &OutputTx,
        metrics_emitter: &Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
    ) {
        // Step 1.5: drain queued announces.
        Self::drain_announce_events(session_manager, session_id).await;
        loop {
            // Get next pending message
            let Some(pending) = session_manager.pop_pending_message(session_id).await else {
                break;
            };

            // Set busy before calling LLM
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs = cs.write().await;
                cs.set_llm_busy(true);
                cs.set_llm_state(LlmState::Requesting);
            }

            // Record pre-call fingerprint for cache-break attribution.
            // Pass actual registered tool names so fingerprint includes
            // the tools dimension (not just the system prompt).
            // Pass provider default headers to activate the HeadersChanged
            // cache break dimension.
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let mut cs_write = cs.write().await;
                let sys = cs_write.system_prompt().map(|s| s.to_string());
                let tool_names: Option<Vec<String>> =
                    match session_manager.get_tool_registry().await {
                        Some(tr) => Some(tr.list_tool_names().await),
                        None => None,
                    };
                let tools_ref: Option<&[String]> = tool_names.as_deref();
                let headers_pairs: Vec<(String, String)> = cs_write
                    .llm_caller()
                    .map(|c| c.default_header_pairs())
                    .unwrap_or_default();
                let headers_refs: Vec<(&str, &str)> = headers_pairs
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                cs_write.record_prompt_fingerprint(sys.as_deref(), tools_ref, Some(&headers_refs));
            }

            // Non-streaming path: delegate to ConversationSession.
            let turn_start = Instant::now();
            let result: Result<StreamResult, closeclaw_llm::LLMError> = {
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    cs.read()
                        .await
                        .invoke_llm(&pending.content)
                        .await
                        .map(Into::into)
                } else {
                    Err(closeclaw_llm::LLMError::InvalidRequest(
                        "session not found".to_string(),
                    ))
                }
            };
            let turn_metrics = TurnMetrics {
                turn_duration_ms: turn_start.elapsed().as_millis() as u64,
            };
            let skip_drain = Self::clear_busy_and_send(
                session_manager,
                session_id,
                result,
                turn_metrics,
                output_tx,
                metrics_emitter,
            )
            .await;
            if skip_drain {
                tracing::info!(
                    session_id = %session_id,
                    "drain_pending_loop: recovery action requested stop, breaking drain loop"
                );
                break;
            }
        }
    }

    /// Handle a recovery action from the health check pipeline.
    ///
    /// Returns `true` if the caller should skip `drain_pending_loop`.
    fn handle_recovery_action(
        session_id: &str,
        action: RecoverableAction,
        output_tx: &OutputTx,
    ) -> bool {
        match action {
            RecoverableAction::NotifyUser { message } => {
                tracing::warn!(
                    session_id,
                    message = %message,
                    "health check: sending recovery notification to user"
                );
                let output_tx = output_tx.clone();
                let msg = message.clone();
                tokio::spawn(async move {
                    let guard = output_tx.read().await;
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.send((msg, vec![])).await;
                    }
                });
                false
            }
            RecoverableAction::Stop { reason } => {
                tracing::warn!(
                    session_id,
                    reason = %reason,
                    "health check: Stop action — skipping pending drain"
                );
                true
            }
            RecoverableAction::Retry {
                delay_ms,
                instruction,
            } => {
                // TODO: Implement retry with backoff. Requires session execution
                // loop integration — retry involves re-invoking LLM with optional
                // instruction injection and backoff delay, which is beyond gateway
                // layer scope. Track with follow-up issue.
                tracing::warn!(
                    session_id,
                    delay_ms,
                    instruction = ?instruction,
                    "health check: Retry action — not yet implemented (TODO)"
                );
                false
            }
        }
    }

    /// Test-only wrapper to expose `handle_recovery_action` for unit tests.
    #[cfg(test)]
    pub(super) fn test_handle_recovery_action(
        session_id: &str,
        action: RecoverableAction,
        output_tx: &OutputTx,
    ) -> bool {
        Self::handle_recovery_action(session_id, action, output_tx)
    }

    /// Step 1.5: best-effort announce to parent (run-mode child).
    ///
    /// Invoked at the end of `clear_busy_and_send` so a finished
    /// run-mode child session can notify its parent that new output
    /// is available. Wraps `SessionManager::try_push_announce`.
    pub(super) async fn maybe_push_announce(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) {
        session_manager.try_push_announce(session_id).await;
    }

    /// Step 1.5: drain queued announces before processing the next
    /// pending message.
    ///
    /// Invoked at the start of `drain_pending_loop` so any announces
    /// queued during the previous LLM turn are injected into the
    /// session before the next pending user message is processed.
    /// Wraps `SessionManager::drain_and_inject_announces`.
    pub(super) async fn drain_announce_events(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) {
        session_manager.drain_and_inject_announces(session_id).await;

        // Drain background task completion notifications and inject as
        // system messages so the agent sees them on the next turn.
        let Some(tm) = session_manager.get_task_manager().await else {
            return;
        };
        let notifications = tm.drain_notifications().await;
        if notifications.is_empty() {
            return;
        }
        let Some(cs) = session_manager.get_conversation_session(session_id).await else {
            tracing::warn!(
                session_id = %session_id,
                "drain_announce_events: session not found for task notifications"
            );
            return;
        };
        let mut cs_write = cs.write().await;
        for notif in notifications {
            let prefix = match notif.priority {
                NotificationPriority::Now => "[🚨 紧急] 后台任务",
                NotificationPriority::Next => "[⚠️ 需立即处理] 后台任务",
                NotificationPriority::Later => "[后台任务]",
            };
            let text = format!(
                "{} {}。输出文件：{}{}",
                prefix,
                notif.summary,
                notif.output_path.display(),
                notif
                    .suggestion
                    .as_ref()
                    .map(|s| format!("。建议：{}", s))
                    .unwrap_or_default()
            );
            cs_write.inject_system_message(text);
        }
        drop(cs_write);
    }
}
