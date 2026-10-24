//! Announce integration and LLM-finishing helpers for SessionMessageHandler.

use std::sync::Arc;

use super::session_handler::SessionMessageHandler;
use super::OutputTx;
use crate::outbound::StreamResult;
use crate::session_manager::SessionManager;
use crate::Gateway;
use closeclaw_common::MetricsEmitter;
use closeclaw_common::NotificationPriority;
use closeclaw_common::RequestContext;
use closeclaw_llm::resolve_anthropic_effective as resolve_anthropic_effective_shared;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::LLMError;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::run_health::RecoverableAction;
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

/// Check if a model name looks like an Anthropic model.
///
/// All Anthropic production models use the "claude" prefix.
/// This is used as a heuristic when `provider_id` is not available
/// but the model name is the actual model name
/// (not a placeholder like "default").
fn is_anthropic_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.contains("claude") || lower.contains("anthropic")
}

/// Resolve the effective reasoning level for a given model.
///
/// Iterates through the knowledge base to find the provider that
/// serves this model, then checks if the requested level is
/// supported. If not, returns the highest supported level.
///
/// When the model is not in the knowledge base, falls back to
/// provider-specific static capability tables. Currently handles:
/// - **Anthropic**: Off→Low, non-High→High (only High natively supported)
/// - **Default heuristic** (all other unknown models): Max→High, Off→Low,
///   Low/Medium/High unchanged. Conservative fallback to avoid unsupported
///   configurations.
///
/// Provider identification is done via model name heuristic ("claude"
/// or "anthropic" prefix) since `provider_id` is not available at
/// the session level.
pub(crate) fn resolve_effective_reasoning_level(
    model: &str,
    requested: ReasoningLevel,
    knowledge: &closeclaw_llm::ProviderModelKnowledge,
) -> ReasoningLevel {
    for kb_provider_id in knowledge.all_providers() {
        let models = knowledge.all_models(kb_provider_id);
        if models.contains(&model) {
            if let Some(params) = knowledge.find(kb_provider_id, model) {
                return match params.reasoning_levels {
                    closeclaw_llm::knowledge::ReasoningLevels::None => requested,
                    closeclaw_llm::knowledge::ReasoningLevels::Toggle { on: true } => {
                        // Toggle models: Off/Low → disabled injection, Medium/High →
                        // enabled injection, Max → downgraded to High then enabled.
                        if requested == ReasoningLevel::Max {
                            ReasoningLevel::High
                        } else if requested == ReasoningLevel::Off && params.reasoning_always {
                            // Models that always produce reasoning output (e.g. MiMo)
                            // cannot truly disable it — downgrade Off to Low.
                            ReasoningLevel::Low
                        } else {
                            requested
                        }
                    }
                    closeclaw_llm::knowledge::ReasoningLevels::Toggle { on: false } => {
                        ReasoningLevel::Low
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
    // Model not in knowledge base — apply provider-specific fallback.
    // Use model name heuristic ("claude" or "anthropic" prefix).
    if is_anthropic_model(model) {
        return resolve_anthropic_effective_shared(requested);
    }

    // Unknown model — apply default heuristic fallback.
    // Conservative: Max may not be supported, Off may not be closable.
    match requested {
        ReasoningLevel::Max => ReasoningLevel::High,
        ReasoningLevel::Off => ReasoningLevel::Low,
        _ => requested,
    }
}

/// Turn-level timing metadata passed through the health
/// check pipeline so hard rules receive actual runtime values.
pub(super) struct TurnMetrics {
    pub turn_duration_ms: u64,
}

impl SessionMessageHandler {
    /// Clear busy flag, send output, and drain pending messages.
    pub(super) async fn finish_llm(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<StreamResult, LLMError>,
        turn_start: Instant,
        output_tx: &OutputTx,
        metrics_emitter: &Option<Arc<dyn MetricsEmitter>>,
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
        Self::drain_pending_loop(
            session_manager,
            session_id,
            output_tx,
            metrics_emitter,
            gateway,
        )
        .await;

        // Step 1.3: idle→verify hook — inject verify message when session
        // becomes idle during workflow execution.
        super::idle_verify_hook::maybe_inject_workflow_verify(session_manager, session_id, gateway)
            .await;

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
        result: Result<StreamResult, LLMError>,
        turn_metrics: TurnMetrics,
        output_tx: &OutputTx,
        metrics_emitter: &Option<Arc<dyn MetricsEmitter>>,
        gateway: Option<&Arc<crate::Gateway>>,
    ) -> bool {
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            let cs = cs.write().await;
            cs.set_llm_busy(false);
            cs.set_llm_state(LlmState::Idle);
        }
        let mut skip_drain = false;
        match result {
            Ok(mut stream_result) => {
                // Dispatch tool calls when response contains ToolUse blocks.
                // Replaces ToolUse blocks with executed ToolResult blocks.
                stream_result.content_blocks = Self::maybe_dispatch_tool_calls(
                    session_manager,
                    session_id,
                    stream_result.content_blocks,
                    &session_manager.file_mutex_map,
                    gateway,
                )
                .await;
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
                    // Cache hit consistency check + usage accumulation
                    // at the unified advancement point (extracted to
                    // session_handler_cache_check.rs). The debug_log
                    // instance comes from the gateway so the disposal
                    // block can emit the structured anomaly event.
                    let debug_log = gateway.and_then(|gw| gw.get_debug_log());
                    crate::session_handler_cache_check::check_cache_anomaly_and_accumulate_usage(
                        session_id,
                        &mut cs_write,
                        &stream_result,
                        metrics_emitter,
                        debug_log.as_ref(),
                    );

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
                deliver_and_signal_turn(gateway, session_id, output_tx, stream_result).await;
                // Send pending workflow blocked notification to owner (Step 1.6).
                Self::drain_workflow_notification(session_manager, session_id, gateway).await;
            }
            Err(err) => {
                tracing::warn!(session_id, error = %err, "LLM call failed");
                // Write complete Thinking blocks from partial streaming
                // error to conversation history (design-doc §F2/§F5).
                Self::write_partial_thinking(session_manager, session_id, &err).await;
                // Mark run-mode child as Errored so try_push_announce
                // resolves the correct ChildCompletionStatus.
                session_manager.notify_child_error(session_id).await;
                // Empty payload: callers must not block until their
                // timeout when the turn failed.
                emit_turn_completion(output_tx, String::new(), Vec::new()).await;
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
        metrics_emitter: &Option<Arc<dyn MetricsEmitter>>,
        gateway: Option<&Arc<Gateway>>,
    ) {
        Self::drain_announces_rest(session_manager, session_id, gateway).await;
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

            // Non-streaming path with pre-call reasoning resolution.
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read()
                    .await
                    .set_request_context(RequestContext::default());
            }
            super::session_handler_reasoning::resolve_before_llm_call(session_manager, session_id)
                .await;
            let turn_start = Instant::now();
            let result: Result<StreamResult, LLMError> = {
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    super::session_handler_dispatch::invoke_llm_with_call_target(
                        &cs,
                        &pending.content,
                        false,
                    )
                    .await
                } else {
                    Err(LLMError::InvalidRequest("session not found".to_string()))
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

    /// Handle recovery action; uses `Box::pin` to break recursive async cycle.
    fn handle_recovery_action(
        session_manager: Arc<SessionManager>,
        session_id: String,
        action: RecoverableAction,
        output_tx: OutputTx,
        metrics_emitter: Option<Arc<dyn MetricsEmitter>>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> {
        Box::pin(Self::handle_recovery_action_impl(
            session_manager,
            session_id,
            action,
            output_tx,
            metrics_emitter,
        ))
    }

    async fn handle_recovery_action_impl(
        session_manager: Arc<SessionManager>,
        session_id: String,
        action: RecoverableAction,
        output_tx: OutputTx,
        metrics_emitter: Option<Arc<dyn MetricsEmitter>>,
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

    /// Handle NotifyUser: inject transcript + send message to user.
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
        metrics_emitter: Option<Arc<dyn MetricsEmitter>>,
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
        // 3. Re-invoke LLM with pre-call reasoning resolution.
        if let Some(cs) = session_manager.get_conversation_session(&session_id).await {
            cs.read()
                .await
                .set_request_context(RequestContext::default());
        }
        super::session_handler_reasoning::resolve_before_llm_call(&session_manager, &session_id)
            .await;
        let result: Result<StreamResult, LLMError> = {
            if let Some(cs) = session_manager.get_conversation_session(&session_id).await {
                // Health-check Retry never pre-appends the invoked
                // content to history (empty invoke content; only an
                // optional system-message injection above), so the
                // declaration stays false.
                super::session_handler_dispatch::invoke_llm_with_call_target(&cs, "", false).await
            } else {
                Err(LLMError::InvalidRequest(
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
        metrics_emitter: &'a Option<Arc<dyn MetricsEmitter>>,
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

    /// Drain pending workflow blocked notification and send to owner.
    /// outbound channel (IM plugin).
    pub(super) async fn drain_workflow_notification(
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
        let Some(channel) = session_manager.session_channel(session_id).await else {
            return;
        };
        if let Err(e) = gw
            .send_outbound_simplified(&chat_id, &channel, &notif.message)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "failed to send workflow blocked notification via simplified outbound"
            );
        }
    }

    /// Drain all priority announces at turn end.
    ///
    /// All priorities (Now / Next / Later) are drained in a single pass.
    /// Priority only determines queue ordering and display prefix, not
    /// injection timing (design-doc §通知机制).
    pub(super) async fn drain_announces_rest(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        gateway: Option<&Arc<Gateway>>,
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
        // notifications) with all priorities. Priority only determines
        // queue ordering and display prefix (design-doc §通知机制).
        session_manager
            .drain_and_inject_announces_filtered(
                session_id,
                |p| *p <= NotificationPriority::Now,
                gateway,
            )
            .await;
    }

    /// Inject running task summary as a system message.
    async fn inject_running_tasks_summary(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        running_tasks: &[closeclaw_common::RunningTaskInfo],
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

/// Assemble the outbound text of a finished turn, deliver it in batch
/// mode (outbound-flow.md), and emit the turn-completion signal.
///
/// Shared tail for the `Ok` arm of `clear_busy_and_send`; the `Err`
/// arm emits the same signal directly via [`emit_turn_completion`]
/// with an empty payload.
async fn deliver_and_signal_turn(
    gateway: Option<&Arc<Gateway>>,
    session_id: &str,
    output_tx: &OutputTx,
    stream_result: StreamResult,
) {
    let text = stream_result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    // Batch outbound (outbound-flow.md batch mode): non-streaming
    // turns are sent to the IM Adapter here; streaming turns were
    // already delivered incrementally by send_outbound_streaming.
    if let Some(gw) = gateway {
        crate::outbound_helpers::deliver_batch_result(
            gw,
            session_id,
            &text,
            &stream_result.content_blocks,
        )
        .await;
    }
    // Turn-completion signal for callers (CLI REPL waits on this
    // channel; the daemon chat wiring uses it to finalize turns).
    emit_turn_completion(output_tx, text, stream_result.content_blocks).await;
}

/// Turn-completion signal: hand `(text, blocks)` to the consumer
/// wired on the output channel (the daemon chat wiring finalizes
/// turns on it). Failure turns send an empty payload so callers do
/// not block until their timeout.
async fn emit_turn_completion(output_tx: &OutputTx, text: String, blocks: Vec<ContentBlock>) {
    let guard = output_tx.read().await;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send((text, blocks)).await;
    }
}

#[cfg(test)]
#[path = "session_handler_announce_tests.rs"]
mod session_handler_announce_tests;
