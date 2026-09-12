//! Idle → verify hook for workflow execution.
//!
//! When a session becomes idle during workflow execution, this module
//! injects a verify message to prompt the agent to check its output
//! against the current step's verification criteria.
//!
//! Extracted from `session_handler_announce.rs` to keep files under the
//! 1000-line project limit. Pure refactoring — behavior unchanged.

use std::sync::Arc;

use super::session_handler::SessionMessageHandler;
use crate::session_manager::SessionManager;

/// Parameters extracted from the handler for verify injection.
pub(crate) struct VerifyInjectParams {
    pub current_step: usize,
    pub allow_blocked: bool,
    pub verify_retry_limit: usize,
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
pub(crate) async fn maybe_inject_workflow_verify(
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
    let Some(params) = check_idle_verify_conditions(&cs_write, session_id) else {
        return;
    };

    // Remove previous verify, build and inject new one.
    inject_verify_message(&mut cs_write, &params);

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
    SessionMessageHandler::drain_workflow_notification(session_manager, session_id, gateway).await;
}

/// Check whether the idle→verify hook should fire.
///
/// Returns `Some(VerifyInjectParams)` if the session is idle and
/// the workflow handler is in Executing phase, `None` otherwise.
pub(crate) fn check_idle_verify_conditions(
    cs: &closeclaw_session::llm_session::ConversationSession,
    session_id: &str,
) -> Option<VerifyInjectParams> {
    let dims = cs.activity_dimensions();
    let is_all_inactive = !dims.any_active();
    if !is_all_inactive {
        tracing::debug!(
            session_id = %session_id,
            llm_active = dims.llm_active,
            fg_tool_active = dims.foreground_tool_active,
            bg_tool_active = dims.background_tool_active,
            child_active = dims.child_active,
            "idle hook: session still active, skipping verify injection"
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

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use closeclaw_session::llm_session::ChatSession;
    use closeclaw_session::workflow_handler::WorkflowHandler;
    use closeclaw_workflow::definition::{Step, Workflow};
    use closeclaw_workflow::run::{GoalHint, Phase, WorkflowRun};

    // ── test-only wrappers ────────────────────────────────────────

    pub(crate) fn test_check_idle_verify_conditions(
        cs: &closeclaw_session::llm_session::ConversationSession,
        session_id: &str,
    ) -> Option<super::VerifyInjectParams> {
        super::check_idle_verify_conditions(cs, session_id)
    }

    pub(crate) fn test_inject_verify_message(
        cs: &mut closeclaw_session::llm_session::ConversationSession,
        params: &super::VerifyInjectParams,
    ) {
        super::inject_verify_message(cs, params)
    }

    /// Test wrapper: expose `maybe_inject_workflow_verify` for integration tests.
    pub(crate) async fn test_maybe_inject_workflow_verify(
        session_manager: &Arc<crate::SessionManager>,
        session_id: &str,
        gateway: Option<&Arc<crate::Gateway>>,
    ) {
        super::maybe_inject_workflow_verify(session_manager, session_id, gateway).await;
    }

    // ── helpers ──────────────────────────────────────────────────

    pub(crate) fn make_test_workflow() -> Workflow {
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

    pub(crate) fn make_test_run(phase: Phase, pending_verify: usize) -> WorkflowRun {
        WorkflowRun {
            workflow_id: "test-wf".to_string(),
            definition_name: "Test Workflow".to_string(),
            definition_version: "0.1".to_string(),
            current_step: 0,
            phase,
            step_history: vec![],
            step_data: serde_yaml::Value::Null,
            pending_goal_hint: GoalHint::default(),
            pending_verify,
            paused_reason: String::new(),
        }
    }

    pub(crate) fn make_session_with_handler(
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
        for phase in [Phase::Jumping, Phase::Blocked, Phase::Complete] {
            let cs = make_session_with_handler(phase.clone(), 0);
            let result = test_check_idle_verify_conditions(&cs, "sid");
            assert!(result.is_none(), "phase {:?} should return None", phase);
        }
    }

    #[test]
    fn test_check_conditions_idle_verifying_returns_params() {
        let cs = make_session_with_handler(Phase::Verifying, 0);
        let result = test_check_idle_verify_conditions(&cs, "sid");
        let params = result.expect("should return Some for idle+verifying");
        assert_eq!(params.current_step, 0);
        assert!(params.allow_blocked);
        assert_eq!(params.verify_retry_limit, 3);
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
        use closeclaw_common::ContentBlock;

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
        assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Verifying);
    }

    #[test]
    fn test_verify_counter_blocks_at_limit() {
        let mut cs = make_session_with_handler(Phase::Executing, 0);
        {
            let handler = cs.workflow_handler_mut().unwrap();
            handler.on_verify_injected(1); // 1st: pending=1, limit=1 → blocked
        }
        assert_eq!(cs.workflow_handler().unwrap().run().pending_verify, 1);
        assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Blocked);
    }

    // ── Four-dimensional idle check (Step 1.3) ──────────────────────────

    /// background_tool_active=true → not all inactive → None.
    #[test]
    fn test_check_conditions_bg_tool_active_returns_none() {
        let cs = make_session_with_handler(Phase::Executing, 0);
        cs.register_tool_call("bg-1", "bash", "ls");
        cs.update_tool_state("bg-1", closeclaw_common::ToolExecState::RunningBackground);
        let result = test_check_idle_verify_conditions(&cs, "sid");
        assert!(
            result.is_none(),
            "bg_tool_active must prevent verify injection"
        );
    }

    /// child_active=true → not all inactive → None.
    #[test]
    fn test_check_conditions_child_active_returns_none() {
        let cs = make_session_with_handler(Phase::Executing, 0);
        cs.register_child("child-1", "agent-a", "task");
        let result = test_check_idle_verify_conditions(&cs, "sid");
        assert!(
            result.is_none(),
            "child_active must prevent verify injection"
        );
    }

    /// Only background_tool_active true, all others false → None.
    #[test]
    fn test_check_conditions_only_bg_tool_active_returns_none() {
        let cs = make_session_with_handler(Phase::Executing, 0);
        // LLM is Idle by default, no foreground tools, no children.
        cs.register_tool_call("bg-only", "bash", "ls");
        cs.update_tool_state(
            "bg-only",
            closeclaw_common::ToolExecState::RunningBackground,
        );
        let dims = cs.activity_dimensions();
        assert!(!dims.llm_active);
        assert!(!dims.foreground_tool_active);
        assert!(dims.background_tool_active);
        assert!(!dims.child_active);
        let result = test_check_idle_verify_conditions(&cs, "sid");
        assert!(
            result.is_none(),
            "only bg_tool_active must still return None (not all inactive)"
        );
    }

    /// All four dimensions inactive + Executing → Some.
    #[test]
    fn test_check_conditions_all_inactive_executing_returns_params() {
        let cs = make_session_with_handler(Phase::Executing, 0);
        let dims = cs.activity_dimensions();
        assert!(!dims.any_active(), "all dimensions should be inactive");
        let result = test_check_idle_verify_conditions(&cs, "sid");
        let params = result.expect("all inactive + Executing should return Some");
        assert_eq!(params.current_step, 0);
    }

    /// child_active=true + Executing → None (previously would have
    /// returned Some because exec_status was Idle with child running).
    #[test]
    fn test_check_conditions_child_active_prev_false_positive() {
        let cs = make_session_with_handler(Phase::Executing, 0);
        cs.register_child("child-fp", "agent-a", "task");
        // Verify exec_status would be Idle (old behavior) but
        // activity_dimensions catches child_active.
        assert_eq!(
            cs.exec_status(),
            closeclaw_common::SessionExecStatus::Idle,
            "child alone does not affect exec_status"
        );
        let dims = cs.activity_dimensions();
        assert!(dims.child_active);
        let result = test_check_idle_verify_conditions(&cs, "sid");
        assert!(
            result.is_none(),
            "child_active must prevent verify (four-dimensional check)"
        );
    }

    /// background_tool_active=true + Executing → None (previously would
    /// have returned Some because exec_status was Idle with bg tool).
    #[test]
    fn test_check_conditions_bg_tool_prev_false_positive() {
        let cs = make_session_with_handler(Phase::Executing, 0);
        cs.register_tool_call("bg-fp", "bash", "ls");
        cs.update_tool_state("bg-fp", closeclaw_common::ToolExecState::RunningBackground);
        assert_eq!(
            cs.exec_status(),
            closeclaw_common::SessionExecStatus::Idle,
            "bg tool alone does not affect exec_status"
        );
        let dims = cs.activity_dimensions();
        assert!(dims.background_tool_active);
        let result = test_check_idle_verify_conditions(&cs, "sid");
        assert!(
            result.is_none(),
            "bg_tool_active must prevent verify (four-dimensional check)"
        );
    }
}
