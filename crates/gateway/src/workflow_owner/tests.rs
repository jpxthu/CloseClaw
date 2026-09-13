//! Unit tests for `apply_resolve_action` (Step 1.3).

use closeclaw_common::ContentBlock;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::workflow_handler::WorkflowHandler;
use closeclaw_workflow::definition::{Step, Workflow};
use closeclaw_workflow::run::{GoalHint, Phase, WorkflowRun};

use super::Gateway;

// ── helpers ────────────────────────────────────────────────────────────

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
        current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::default(),
        pending_verify: closeclaw_workflow::run::PendingVerify {
            count: pending_verify,
            ..Default::default()
        },
        paused_reason: String::new(),
    }
}

fn make_session(
    phase: Phase,
    pending_verify: usize,
) -> closeclaw_session::llm_session::ConversationSession {
    let mut cs = closeclaw_session::llm_session::ConversationSession::new(
        "test-sid".to_string(),
        "model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    let handler = WorkflowHandler::new(make_test_run(phase, pending_verify), make_test_workflow());
    cs.set_workflow_handler(Some(handler));
    cs
}

/// Extract text content from a session message.
fn message_text(msg: &closeclaw_session::llm_session::SessionMessage) -> String {
    msg.content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .next()
        .unwrap_or_default()
}

// ── Tests ──────────────────────────────────────────────────────────────

/// After resolve: goal message preserved, old verify cleaned, new verify
/// injected.
#[test]
fn test_resolve_preserves_goal_cleans_verify_injects_new() {
    let mut cs = make_session(Phase::Blocked, 3);
    // Inject goal and old verify messages.
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
    cs.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");

    Gateway::apply_resolve_action(&mut cs);

    let messages = cs.messages();
    // Expect: goal + new verify = 2 workflow messages.
    assert_eq!(messages.len(), 2, "goal + new verify");
    assert_eq!(messages[0].role, "workflow");
    assert!(message_text(&messages[0]).starts_with("[workflow goal]"));
    assert_eq!(messages[1].role, "workflow");
    assert!(message_text(&messages[1]).starts_with("Verify Step"));
    // Old verify should be gone, new one present.
    let old_verify = "Verify Step 0 (Step 0):\nCheck output";
    let texts: Vec<String> = messages.iter().map(|m| message_text(m)).collect();
    assert_eq!(
        texts[1],
        closeclaw_workflow::definition::build_verify_message(
            &cs.workflow_handler().unwrap().definition().steps[0],
            true,
        )
    );
    // Old verify text must not appear.
    assert!(!texts.contains(&old_verify.to_string()));
}

/// Goal message content is consistent before and after resolve.
#[test]
fn test_resolve_goal_content_unchanged() {
    let mut cs = make_session(Phase::Blocked, 3);
    let goal_text = "[workflow goal] Step 0: Step 0\n\nDo first thing";
    cs.inject_workflow_message(goal_text);
    cs.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");

    let goal_before = message_text(&cs.messages()[0]);
    Gateway::apply_resolve_action(&mut cs);

    let messages = cs.messages();
    let goal_after = message_text(&messages[0]);
    assert_eq!(goal_before, goal_after, "goal content must be unchanged");
}

/// pending_verify is zeroed and not overwritten by stale snapshot.
#[test]
fn test_resolve_pending_verify_zeroed_not_overwritten() {
    let mut cs = make_session(Phase::Blocked, 3);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    // Before resolve: pending_verify = 3.
    assert_eq!(cs.workflow_handler().unwrap().run().pending_verify.count, 3);
    assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Blocked);

    Gateway::apply_resolve_action(&mut cs);

    // After resolve: pending_verify = 0, phase = Verifying.
    let handler = cs.workflow_handler().unwrap();
    assert_eq!(
        handler.run().pending_verify.count,
        0,
        "pending_verify must be zeroed"
    );
    assert_eq!(
        handler.run().phase,
        Phase::Verifying,
        "phase must be Verifying"
    );
}

/// After resolve with user messages interleaved, user messages are
/// preserved alongside goal and new verify.
#[test]
fn test_resolve_preserves_user_messages() {
    let mut cs = make_session(Phase::Blocked, 3);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
    cs.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
    cs.append_transcript(
        "user",
        vec![ContentBlock::Text("please continue".to_string())],
    );

    Gateway::apply_resolve_action(&mut cs);

    let messages = cs.messages();
    // goal + user + new verify = 3.
    assert_eq!(messages.len(), 3, "goal + user + new verify");
    assert_eq!(messages[0].role, "workflow"); // goal
    assert!(message_text(&messages[0]).starts_with("[workflow goal]"));
    assert_eq!(messages[1].role, "user");
    assert_eq!(message_text(&messages[1]), "please continue");
    assert_eq!(messages[2].role, "workflow"); // new verify
    assert!(message_text(&messages[2]).starts_with("Verify Step"));
}

/// After resolve with no old verify messages, new verify is still injected.
#[test]
fn test_resolve_no_old_verify_still_injects_new() {
    let mut cs = make_session(Phase::Blocked, 0);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    Gateway::apply_resolve_action(&mut cs);

    let messages = cs.messages();
    // goal + new verify = 2.
    assert_eq!(messages.len(), 2, "goal + new verify");
    assert!(message_text(&messages[1]).starts_with("Verify Step"));
}
