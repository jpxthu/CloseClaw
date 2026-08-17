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
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::run_health::RecoverableAction;
use closeclaw_tasks::NotificationPriority;
use tokio::time::Instant;

/// Resolve effective level for a multi-level provider.
///
/// Picks the highest supported level that is ≤ `requested`.
fn resolve_for_levels(
    requested: ReasoningLevel,
    off: bool,
    base: bool,
    reasoner: bool,
) -> ReasoningLevel {
    match requested {
        ReasoningLevel::Max if reasoner => ReasoningLevel::Max,
        ReasoningLevel::Max => ReasoningLevel::High,
        ReasoningLevel::High if reasoner => ReasoningLevel::High,
        ReasoningLevel::High => {
            if base {
                ReasoningLevel::Medium
            } else if off {
                ReasoningLevel::Low
            } else {
                ReasoningLevel::High
            }
        }
        ReasoningLevel::Medium if base => ReasoningLevel::Medium,
        ReasoningLevel::Medium => {
            if off {
                ReasoningLevel::Low
            } else {
                ReasoningLevel::Medium
            }
        }
        ReasoningLevel::Off if off => ReasoningLevel::Off,
        ReasoningLevel::Off => ReasoningLevel::Low,
        ReasoningLevel::Low if off => ReasoningLevel::Low,
        ReasoningLevel::Low => ReasoningLevel::Low,
    }
}

/// Resolve the effective reasoning level for a given model.
///
/// Iterates through the knowledge base to find the provider that
/// serves this model, then checks if the requested level is
/// supported. If not, returns the highest supported level.
/// Falls back to `requested` when the model is not in the knowledge base.
fn resolve_effective_reasoning_level(
    model: &str,
    requested: ReasoningLevel,
    knowledge: &closeclaw_llm::ProviderModelKnowledge,
) -> ReasoningLevel {
    for provider_id in knowledge.all_providers() {
        let models = knowledge.all_models(provider_id);
        if models.contains(&model) {
            if let Some(params) = knowledge.find(provider_id, model) {
                return match params.reasoning_levels {
                    closeclaw_llm::knowledge::ReasoningLevels::None => requested,
                    closeclaw_llm::knowledge::ReasoningLevels::Toggle { .. } => {
                        ReasoningLevel::High
                    }
                    closeclaw_llm::knowledge::ReasoningLevels::Levels {
                        off,
                        base,
                        reasoner,
                    } => resolve_for_levels(requested, off, base, reasoner),
                };
            }
        }
    }
    requested
}

/// Turn-level timing metadata passed through the health
/// check pipeline so hard rules receive actual runtime values.
pub(super) struct TurnMetrics {
    pub turn_duration_ms: u64,
}

/// Parameters extracted from the handler for verify injection.
struct VerifyInjectParams {
    current_step: usize,
    allow_blocked: bool,
    verify_retry_limit: usize,
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
        gateway: Option<&Arc<crate::Gateway>>,
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
            gateway,
        )
        .await;

        // Step 1.5: Skip drain if recovery action requested stop.
        if skip_drain {
            tracing::info!(
                session_id = %session_id,
                "finish_llm: recovery action requested stop, skipping pending drain"
            );
            return;
        }

        // Note: yield no longer prevents drain. During yield, user
        // messages are injected directly into the conversation history
        // (not queued), so the LLM processes them immediately. After
        // the turn completes, drain_pending_loop processes any remaining
        // queued announce events or pending messages normally.
        Self::drain_pending_loop(session_manager, session_id, output_tx, metrics_emitter).await;

        // Step 1.3: idle→verify hook — inject verify message when session
        // becomes idle during workflow execution.
        Self::maybe_inject_workflow_verify(session_manager, session_id, gateway).await;

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

    /// Returns `true` if the caller should skip `drain_pending_loop`
    /// (recovery action requested a stop).
    async fn clear_busy_and_send(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<StreamResult, closeclaw_llm::LLMError>,
        turn_metrics: TurnMetrics,
        output_tx: &OutputTx,
        metrics_emitter: &Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
        gateway: Option<&Arc<crate::Gateway>>,
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
                    // Process workflow tool results (Step 1.6).
                    // Routes workflow actions to the engine and queues
                    // blocked notifications for owner delivery.
                    cs_write.process_workflow_tool_results(&stream_result.content_blocks);
                    // Cache break detection (must run before accumulate_usage
                    // so that last_cache_read_tokens still holds the previous value).
                    if let Some(info) = cs_write.detect_cache_break_for_usage(
                        stream_result.usage.cache_read_tokens,
                        Some(stream_result.usage.prompt_tokens),
                    ) {
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
                        cs_write.push_system_notification(
                            info.format_notification(),
                            NotificationPriority::Now,
                        );
                    }
                    cs_write.accumulate_usage(&stream_result.usage);

                    // Resolve effective reasoning level (post-provider-downgrade).
                    if let Some(knowledge) = gateway.and_then(|g| g.model_knowledge()) {
                        let effective = resolve_effective_reasoning_level(
                            cs_write.model(),
                            cs_write.reasoning_level(),
                            knowledge,
                        );
                        cs_write.set_effective_reasoning_level(effective);
                    }

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
                        skip_drain = Self::handle_recovery_action(
                            Arc::clone(session_manager),
                            session_id.to_string(),
                            action,
                            output_tx.clone(),
                            metrics_emitter.clone(),
                        )
                        .await;
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
                // Send pending workflow blocked notification to owner (Step 1.6).
                Self::drain_workflow_notification(session_manager, session_id, gateway).await;
            }
            Err(err) => {
                tracing::warn!(session_id, error = %err, "LLM call failed");
                // Mark run-mode child as Errored so try_push_announce
                // resolves the correct ChildCompletionStatus.
                session_manager.notify_child_error(session_id).await;
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
        // Step 1.4: drain Next/Later priority announces at turn start.
        // Now-priority events were already drained before the LLM call.
        Self::drain_announces_rest(session_manager, session_id).await;
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

            // Non-streaming path: delegate to ConversationSession.
            // Set default request context (no inbound metadata for queued messages).
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read()
                    .await
                    .set_request_context(closeclaw_common::RequestContext::default());
            }
            let turn_start = Instant::now();
            let result: Result<StreamResult, closeclaw_llm::LLMError> = {
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    cs.write()
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
                None,
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
    ///
    /// Uses `Box::pin` to break the recursive async call cycle:
    /// `handle_recovery_action` → `clear_busy_and_send` → `handle_recovery_action`.
    fn handle_recovery_action(
        session_manager: Arc<SessionManager>,
        session_id: String,
        action: RecoverableAction,
        output_tx: OutputTx,
        metrics_emitter: Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> {
        Box::pin(Self::handle_recovery_action_impl(
            session_manager,
            session_id,
            action,
            output_tx,
            metrics_emitter,
        ))
    }

    /// Inner implementation of recovery action handling.
    async fn handle_recovery_action_impl(
        session_manager: Arc<SessionManager>,
        session_id: String,
        action: RecoverableAction,
        output_tx: OutputTx,
        metrics_emitter: Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
    ) -> bool {
        match action {
            RecoverableAction::NotifyUser { message } => {
                Self::handle_notify_user(session_manager, &session_id, message, output_tx).await
            }
            RecoverableAction::Stop { reason } => Self::handle_stop(session_id, reason),
            RecoverableAction::Retry {
                delay_ms,
                instruction,
            } => {
                Self::handle_retry(
                    session_manager,
                    session_id,
                    delay_ms,
                    instruction,
                    output_tx,
                    metrics_emitter,
                )
                .await
            }
        }
    }

    /// Handle NotifyUser: inject transcript + send message to user, don't
    /// skip drain.
    ///
    /// Writes the notification as an assistant message to the transcript
    /// (design-doc §失败类别与处理) and simultaneously sends it to the
    /// user via `output_tx`.
    async fn handle_notify_user(
        session_manager: Arc<SessionManager>,
        session_id: &str,
        message: String,
        output_tx: OutputTx,
    ) -> bool {
        tracing::warn!(
            session_id = %session_id,
            message = %message,
            "health check: sending recovery notification to user"
        );
        // Inject as assistant message into transcript (design-doc requirement).
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            cs.write()
                .await
                .append_transcript("assistant", vec![ContentBlock::Text(message.clone())]);
        } else {
            tracing::warn!(
                session_id = %session_id,
                "handle_notify_user: session not found, skipping transcript injection"
            );
        }
        // Also send to user via output_tx.
        tokio::spawn(async move {
            let guard = output_tx.read().await;
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send((message, vec![])).await;
            }
        });
        false
    }

    /// Handle Stop: skip drain without user notification.
    fn handle_stop(session_id: String, reason: String) -> bool {
        tracing::warn!(
            session_id = %session_id,
            reason = %reason,
            "health check: Stop action — skipping pending drain"
        );
        true
    }

    /// Handle Retry: backoff delay → inject instruction → re-invoke LLM.
    async fn handle_retry(
        session_manager: Arc<SessionManager>,
        session_id: String,
        delay_ms: u64,
        instruction: Option<String>,
        output_tx: OutputTx,
        metrics_emitter: Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
    ) -> bool {
        tracing::warn!(
            session_id = %session_id,
            delay_ms,
            instruction = %instruction.as_deref().unwrap_or(""),
            "health check: Retry action — executing backoff retry"
        );
        // 1. Wait for backoff delay.
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
        // 2. Inject retry instruction if provided.
        if let Some(ref instr) = instruction {
            if let Some(cs) = session_manager.get_conversation_session(&session_id).await {
                let mut cs_write = cs.write().await;
                cs_write.inject_system_message(instr.clone());
                drop(cs_write);
            }
        }
        // 3. Re-invoke LLM. Empty content — conversation history has
        //    the original user request.
        // Set default request context (no inbound metadata for retry).
        let result: Result<StreamResult, closeclaw_llm::LLMError> = {
            if let Some(cs) = session_manager.get_conversation_session(&session_id).await {
                cs.read()
                    .await
                    .set_request_context(closeclaw_common::RequestContext::default());
                cs.write().await.invoke_llm("").await.map(Into::into)
            } else {
                Err(closeclaw_llm::LLMError::InvalidRequest(
                    "session not found for retry".to_string(),
                ))
            }
        };
        // 4. Process result through the normal health-check pipeline.
        let turn_start = tokio::time::Instant::now();
        Self::clear_busy_and_send(
            &session_manager,
            &session_id,
            result,
            TurnMetrics {
                turn_duration_ms: turn_start.elapsed().as_millis() as u64,
            },
            &output_tx,
            &metrics_emitter,
            None,
        )
        .await
    }

    /// Test-only wrapper to expose `handle_recovery_action` for unit tests.
    #[cfg(test)]
    pub(super) fn test_handle_recovery_action<'a>(
        session_manager: &'a Arc<SessionManager>,
        session_id: &'a str,
        action: RecoverableAction,
        output_tx: &'a OutputTx,
        metrics_emitter: &'a Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
        let sm = Arc::clone(session_manager);
        let sid = session_id.to_string();
        let tx = output_tx.clone();
        let me = metrics_emitter.clone();
        Box::pin(async move { Self::handle_recovery_action(sm, sid, action, tx, me).await })
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
        session_manager
            .try_push_announce(session_id, NotificationPriority::Next)
            .await;
    }

    /// Drain pending workflow blocked notification and send to owner (Step 1.6).
    ///
    /// After each LLM turn, checks if the workflow handler has queued a
    /// blocked notification and sends it to the owner via the Gateway's
    /// outbound channel (IM plugin).
    async fn drain_workflow_notification(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        gateway: Option<&Arc<crate::Gateway>>,
    ) {
        let notification = {
            let Some(cs) = session_manager.get_conversation_session(session_id).await else {
                return;
            };
            let mut cs_write = cs.write().await;
            cs_write.take_workflow_notification()
        };
        let Some(notif) = notification else {
            return;
        };
        tracing::info!(
            session_id = %session_id,
            workflow = %notif.workflow_name,
            "sending workflow blocked notification to owner"
        );
        // Inject into agent context so agent sees the notification.
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            cs.write()
                .await
                .inject_system_message(notif.message.clone());
        }
        // Send outbound notification to owner via Gateway.
        let Some(gw) = gateway else {
            return;
        };
        let Some(chat_id) = session_manager.get_chat_id(session_id).await else {
            return;
        };
        let sessions = session_manager.sessions.read().await;
        let Some(session) = sessions.get(session_id) else {
            return;
        };
        if let Err(e) = gw
            .send_outbound_simplified(&chat_id, &session.channel, &notif.message)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "failed to send workflow blocked notification via simplified outbound"
            );
        }
    }

    /// Step 1.3: idle→verify hook — inject verify message when session
    /// becomes idle during workflow execution.
    ///
    /// After the pending queue is drained, checks whether the session
    /// is idle (no LLM activity, no foreground tools) and the workflow
    /// handler reports `on_session_idle` (phase == Executing). When
    /// both conditions hold:
    ///
    /// 1. Removes the previous verify message from the transcript
    ///    (preserving goal/recovered messages).
    /// 2. Injects a new verify message via `inject_workflow_message`.
    /// 3. Increments the verify counter via `on_verify_injected`.
    /// 4. Drains any queued workflow notification (e.g. blocked).
    async fn maybe_inject_workflow_verify(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        gateway: Option<&Arc<crate::Gateway>>,
    ) {
        let Some(cs) = session_manager.get_conversation_session(session_id).await else {
            return;
        };
        let mut cs_write = cs.write().await;

        // Lazily build handler if needed.
        cs_write.ensure_workflow_handler();

        // Check conditions; extract handler state if eligible.
        let Some(params) = Self::check_idle_verify_conditions(&cs_write, session_id) else {
            return;
        };

        // Remove previous verify, build and inject new one.
        Self::inject_verify_message(&mut cs_write, &params);

        // Increment verify counter (may transition to Blocked).
        let phase = {
            let handler = cs_write.workflow_handler_mut().unwrap();
            handler.on_verify_injected(params.verify_retry_limit);
            handler.run().phase.clone()
        };
        tracing::info!(
            session_id = %session_id,
            step = params.current_step,
            ?phase,
            "idle hook: verify message injected"
        );

        // Drop the write lock before draining notifications (which may
        // need to read the session).
        drop(cs_write);

        // Drain any queued workflow notification (e.g. blocked after
        // verify limit exceeded).
        Self::drain_workflow_notification(session_manager, session_id, gateway).await;
    }

    /// Check whether the idle→verify hook should fire.
    ///
    /// Returns `Some(VerifyInjectParams)` if the session is idle and
    /// the workflow handler is in Executing phase, `None` otherwise.
    fn check_idle_verify_conditions(
        cs: &closeclaw_session::llm_session::ConversationSession,
        session_id: &str,
    ) -> Option<VerifyInjectParams> {
        let exec_status = cs.exec_status();
        let is_idle = matches!(
            exec_status,
            closeclaw_common::SessionExecStatus::Idle
                | closeclaw_common::SessionExecStatus::IdleWithBackgroundTasks
        );
        if !is_idle {
            tracing::debug!(
                session_id = %session_id,
                ?exec_status,
                "idle hook: session not idle, skipping verify injection"
            );
            return None;
        }

        let handler = cs.workflow_handler()?;
        if !handler.on_session_idle() {
            tracing::debug!(
                session_id = %session_id,
                "idle hook: workflow not in Executing phase, skipping"
            );
            return None;
        }

        let step = handler.definition().steps.get(handler.run().current_step);
        let step_ref = step?;
        let allow_blocked = step_ref
            .allow_blocked
            .unwrap_or(handler.definition().allow_blocked);

        Some(VerifyInjectParams {
            current_step: handler.run().current_step,
            allow_blocked,
            verify_retry_limit: handler.definition().verify_retry_limit,
        })
    }

    /// Remove old verify message and inject a new one.
    fn inject_verify_message(
        cs: &mut closeclaw_session::llm_session::ConversationSession,
        params: &VerifyInjectParams,
    ) {
        cs.remove_workflow_verify_messages();
        let verify_msg = {
            let handler = cs.workflow_handler().unwrap();
            let step = &handler.definition().steps[params.current_step];
            closeclaw_workflow::definition::build_verify_message(step, params.allow_blocked)
        };
        cs.inject_workflow_message(&verify_msg);
    }

    /// Step 1.4: drain Now-priority announces before user message processing.
    ///
    /// Injects session announces with `NotificationPriority::Now` into the
    /// conversation so the agent sees urgent notifications before the next
    /// LLM call. Task notifications are not drained here — they are always
    /// drained at turn start via [`drain_announces_rest`] since
    /// `TaskManager::drain_notifications` consumes all at once.
    pub(super) async fn drain_announces_now(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) {
        session_manager
            .drain_and_inject_announces_filtered(session_id, |p| *p == NotificationPriority::Now)
            .await;
    }

    /// Step 1.4: drain Next + Later priority announces at turn start.
    ///
    /// Injects session announces with `NotificationPriority::Next` or
    /// `NotificationPriority::Later` and all background task completion
    /// notifications. Called at the start of `drain_pending_loop` after
    /// Now-priority events have already been injected.
    pub(super) async fn drain_announces_rest(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) {
        // Step 1.5: Push background task notifications onto the
        // unified queue before draining, so they follow the same
        // drain path as child session announces.
        if let Some(tm) = session_manager.get_task_manager().await {
            let notifications = tm.drain_notifications().await;
            let running_tasks = tm.list_running_tasks().await;
            if !notifications.is_empty() {
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    let mut cs_write = cs.write().await;
                    for notif in notifications {
                        cs_write.push_background_tool_notification(notif);
                    }
                }
            }
            if !running_tasks.is_empty() {
                Self::inject_running_tasks_summary(session_manager, session_id, &running_tasks)
                    .await;
            }
        }

        // Drain session announces (including the just-pushed task
        // notifications) with Next + Later priority.
        session_manager
            .drain_and_inject_announces_filtered(session_id, |p| *p < NotificationPriority::Now)
            .await;
    }

    /// Inject running task summary into the conversation session as a
    /// system message. Task completion notifications are now routed
    /// through the unified queue (Step 1.5); only the running-task
    /// digest is injected directly here.
    async fn inject_running_tasks_summary(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        running_tasks: &[closeclaw_tasks::RunningTaskInfo],
    ) {
        let Some(cs) = session_manager.get_conversation_session(session_id).await else {
            tracing::warn!(
                session_id = %session_id,
                "inject_running_tasks_summary: session not found"
            );
            return;
        };
        let mut cs_write = cs.write().await;
        let mut text = String::from("[后台任务] 当前运行中的后台任务：");
        for task in running_tasks {
            text.push_str(&format!(
                "\n- {} (ID: {}, 已运行 {} 秒)",
                task.command, task.task_id, task.elapsed_secs
            ));
        }
        cs_write.inject_system_message(text);
        drop(cs_write);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::ContentBlock;
    use closeclaw_llm::ProviderModelKnowledge;
    use closeclaw_session::llm_session::ChatSession;
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_session::workflow_handler::WorkflowHandler;
    use closeclaw_workflow::definition::{Step, Workflow};
    use closeclaw_workflow::run::{Phase, WorkflowRun};

    // ── test-only wrappers (access private functions from parent module) ──

    fn test_check_idle_verify_conditions(
        cs: &closeclaw_session::llm_session::ConversationSession,
        session_id: &str,
    ) -> Option<super::VerifyInjectParams> {
        super::SessionMessageHandler::check_idle_verify_conditions(cs, session_id)
    }

    fn test_inject_verify_message(
        cs: &mut closeclaw_session::llm_session::ConversationSession,
        params: &super::VerifyInjectParams,
    ) {
        super::SessionMessageHandler::inject_verify_message(cs, params)
    }

    // ── helpers ──────────────────────────────────────────────────────

    fn make_test_workflow() -> Workflow {
        Workflow {
            id: "test-wf".to_string(),
            name: "Test Workflow".to_string(),
            description: "A test workflow".to_string(),
            version: Some("0.1".to_string()),
            allow_blocked: false,
            verify_retry_limit: 3,
            step_data_schema: serde_yaml::Value::Null,
            steps: vec![Step {
                id: 0,
                name: "Step 0".to_string(),
                goal: "Do first thing".to_string(),
                verify: vec!["Check output".to_string()],
                jump: vec![],
                transitions: vec![],
                allow_blocked: Some(true),
            }],
        }
    }

    fn make_test_run(phase: Phase, pending_verify: usize) -> WorkflowRun {
        WorkflowRun {
            workflow_id: "test-wf".to_string(),
            definition_name: "Test Workflow".to_string(),
            definition_version: "0.1".to_string(),
            current_step: 0,
            phase,
            step_history: vec![],
            step_data: serde_yaml::Value::Null,
            pending_verify,
        }
    }

    fn make_session_with_handler(
        phase: Phase,
        pending_verify: usize,
    ) -> closeclaw_session::llm_session::ConversationSession {
        let mut cs = closeclaw_session::llm_session::ConversationSession::new(
            "test-sid".to_string(),
            "model".to_string(),
            std::path::PathBuf::from("/tmp"),
        );
        let handler =
            WorkflowHandler::new(make_test_run(phase, pending_verify), make_test_workflow());
        cs.set_workflow_handler(Some(handler));
        cs
    }

    #[test]
    fn test_resolve_effective_level_model_not_found_returns_requested() {
        let kb = ProviderModelKnowledge::new();
        let result = resolve_effective_reasoning_level("unknown-model", ReasoningLevel::Max, &kb);
        assert_eq!(result, ReasoningLevel::Max);
    }

    #[test]
    fn test_resolve_effective_level_none_returns_requested() {
        // With an empty knowledge base, model not found → requested.
        let kb = ProviderModelKnowledge::new();
        let result = resolve_effective_reasoning_level("some-model", ReasoningLevel::Medium, &kb);
        assert_eq!(result, ReasoningLevel::Medium);
    }

    #[test]
    fn test_resolve_effective_level_toggle_maps_to_high() {
        // Toggle models (e.g. glm-5.1) map any requested level to High.
        let kb = ProviderModelKnowledge::new();
        for level in [
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::Max,
        ] {
            let result = resolve_effective_reasoning_level("glm-5.1", level, &kb);
            assert_eq!(
                result,
                ReasoningLevel::High,
                "Toggle should map {:?} → High",
                level
            );
        }
    }

    #[test]
    fn test_resolve_effective_level_levels_all_enabled() {
        // deepseek-v4-flash: off=true, base=true, reasoner=true
        let kb = ProviderModelKnowledge::new();
        assert_eq!(
            resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::Max, &kb),
            ReasoningLevel::Max,
        );
        assert_eq!(
            resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::High, &kb),
            ReasoningLevel::High,
        );
        assert_eq!(
            resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::Medium, &kb),
            ReasoningLevel::Medium,
        );
        assert_eq!(
            resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::Low, &kb),
            ReasoningLevel::Low,
        );
    }

    #[test]
    fn test_resolve_effective_level_levels_no_off() {
        // deepseek-v4-pro: off=false, base=true, reasoner=true
        // Medium+ supported directly; Low falls through to Low (no off support).
        let kb = ProviderModelKnowledge::new();
        assert_eq!(
            resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::Max, &kb),
            ReasoningLevel::Max,
        );
        assert_eq!(
            resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::High, &kb),
            ReasoningLevel::High,
        );
        assert_eq!(
            resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::Medium, &kb),
            ReasoningLevel::Medium,
        );
        assert_eq!(
            resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::Low, &kb),
            ReasoningLevel::Low,
        );
    }

    // ── check_idle_verify_conditions ──────────────────────────────

    #[test]
    fn test_check_conditions_busy_session_returns_none() {
        let cs = make_session_with_handler(Phase::Executing, 0);
        cs.set_llm_state(closeclaw_llm::session_state::LlmState::Requesting);
        let result = test_check_idle_verify_conditions(&cs, "sid");
        assert!(result.is_none());
    }

    #[test]
    fn test_check_conditions_non_executing_phase_returns_none() {
        for phase in [
            Phase::Jumping,
            Phase::Blocked,
            Phase::Complete,
            Phase::Verifying,
        ] {
            let cs = make_session_with_handler(phase.clone(), 0);
            let result = test_check_idle_verify_conditions(&cs, "sid");
            assert!(result.is_none(), "phase {:?} should return None", phase);
        }
    }

    #[test]
    fn test_check_conditions_no_handler_returns_none() {
        let cs = closeclaw_session::llm_session::ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            std::path::PathBuf::from("/tmp"),
        );
        // No workflow handler set.
        assert!(cs.workflow_handler().is_none());
        let result = test_check_idle_verify_conditions(&cs, "sid");
        assert!(result.is_none());
    }

    #[test]
    fn test_check_conditions_idle_executing_returns_params() {
        let cs = make_session_with_handler(Phase::Executing, 0);
        let result = test_check_idle_verify_conditions(&cs, "sid");
        let params = result.expect("should return Some for idle+executing");
        assert_eq!(params.current_step, 0);
        assert!(params.allow_blocked); // Step 0 has allow_blocked: Some(true)
        assert_eq!(params.verify_retry_limit, 3);
    }

    // ── inject_verify_message ─────────────────────────────────────

    #[test]
    fn test_inject_verify_removes_old_and_preserves_goal_and_user() {
        let mut cs = make_session_with_handler(Phase::Executing, 0);
        // Pre-populate transcript: goal, old verify, user message.
        cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
        cs.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
        cs.append_transcript("user", vec![ContentBlock::Text("hello".to_string())]);

        let params = VerifyInjectParams {
            current_step: 0,
            allow_blocked: true,
            verify_retry_limit: 3,
        };
        test_inject_verify_message(&mut cs, &params);

        let messages = cs.messages();
        assert_eq!(messages.len(), 3, "goal + user + new verify");
        assert_eq!(messages[0].role, "workflow"); // goal preserved
        assert_eq!(messages[1].role, "user"); // user preserved
        assert_eq!(messages[2].role, "workflow"); // new verify injected

        // Old verify should be gone.
        let wf_texts: Vec<String> = messages
            .iter()
            .filter(|m| m.role == "workflow")
            .map(|m| {
                m.content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .next()
                    .unwrap_or_default()
            })
            .collect();
        assert!(wf_texts[0].starts_with("[workflow goal]"));
        assert!(wf_texts[1].starts_with("Verify Step"));
        // New verify content should match build_verify_message output.
        let expected = closeclaw_workflow::definition::build_verify_message(
            &cs.workflow_handler().unwrap().definition().steps[0],
            true,
        );
        assert_eq!(wf_texts[1], expected);
    }

    #[test]
    fn test_inject_verify_no_old_verify() {
        let mut cs = make_session_with_handler(Phase::Executing, 0);
        cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

        let params = VerifyInjectParams {
            current_step: 0,
            allow_blocked: true,
            verify_retry_limit: 3,
        };
        test_inject_verify_message(&mut cs, &params);

        let messages = cs.messages();
        assert_eq!(messages.len(), 2, "goal + new verify");
        assert_eq!(messages[0].role, "workflow"); // goal
        assert_eq!(messages[1].role, "workflow"); // new verify
    }

    // ── on_verify_injected counter continuation ────────────────────

    #[test]
    fn test_verify_counter_increments_pending_verify() {
        let mut cs = make_session_with_handler(Phase::Executing, 0);
        {
            let handler = cs.workflow_handler_mut().unwrap();
            handler.on_verify_injected(3);
        }
        assert_eq!(cs.workflow_handler().unwrap().run().pending_verify, 1);
        assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Executing);
    }

    #[test]
    fn test_verify_counter_blocks_at_limit() {
        let mut cs = make_session_with_handler(Phase::Executing, 0);
        {
            let handler = cs.workflow_handler_mut().unwrap();
            handler.on_verify_injected(1); // 1st: pending=1, limit=1 → not yet blocked
        }
        assert_eq!(cs.workflow_handler().unwrap().run().pending_verify, 1);
        assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Executing);

        {
            let handler = cs.workflow_handler_mut().unwrap();
            handler.on_verify_injected(1); // 2nd: pending=2, limit=1 → blocked
        }
        assert_eq!(cs.workflow_handler().unwrap().run().pending_verify, 2);
        assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Blocked);
    }
}
