use crate::llm_session::ConversationSession;
use crate::workflow_handler::WorkflowHandler;
use crate::workflow_port::real_engine_port::test_port;
use closeclaw_common::test_helpers::ensure_dir;
use closeclaw_common::ContentBlock;
use closeclaw_workflow::definition::{Step, Workflow};
use closeclaw_workflow::run::{GoalHint, PendingVerify, Phase, WorkflowRun};

fn make_test_run(definition_name: &str) -> WorkflowRun {
    WorkflowRun {
        workflow_id: "test-wf".to_string(),
        definition_name: definition_name.to_string(),
        definition_version: "0.1".to_string(),
        current_step: 0,
        phase: Phase::Executing,
        current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::default(),
        pending_verify: PendingVerify::default(),
        paused_reason: String::new(),
    }
}

/// Decode a serialized run state into the typed form.
fn typed_run_value(state: &serde_json::Value) -> WorkflowRun {
    serde_json::from_value(state.clone()).unwrap()
}

/// Read the current run state of a session's workflow handler as the typed form.
fn typed_run(session: &ConversationSession) -> WorkflowRun {
    typed_run_value(session.workflow_handler().unwrap().run_state())
}

/// Shared SKILL.md writer: creates `dir/workflows/<workflow_name>/` through
/// the sanctioned `ensure_dir` helper and writes a SKILL.md wrapping `yaml`
/// in front-matter. The per-scenario `write_*_skill_md` helpers below only
/// supply the YAML body.
fn write_skill_md_with_yaml(dir: &std::path::Path, workflow_name: &str, yaml: &str) {
    let wf_dir = dir.join("workflows").join(workflow_name);
    ensure_dir(&wf_dir).unwrap();
    let content = format!("---\n{yaml}\n---\n\nBody.\n");
    std::fs::write(wf_dir.join("SKILL.md"), content).unwrap();
}

/// Write a single-step workflow SKILL.md with a default complete transition.
fn write_skill_md(dir: &std::path::Path, workflow_name: &str) {
    let yaml = concat!(
        "id: test-wf\n",
        "name: Test Workflow\n",
        "description: A test workflow\n",
        "steps:\n",
        "  - id: 0\n",
        "    name: Step 0\n",
        "    goal: Do first thing\n",
        "    allow_blocked: true\n",
        "    verify:\n",
        "      - Check output\n",
        "    transitions:\n",
        "      - action: complete",
    );
    write_skill_md_with_yaml(dir, workflow_name, yaml);
}

/// Write a two-step workflow SKILL.md with a default goto transition.
fn write_two_step_skill_md(dir: &std::path::Path, workflow_name: &str) {
    let yaml = concat!(
        "id: test-wf\n",
        "name: Test WF\n",
        "description: test\n",
        "steps:\n",
        "  - id: 0\n",
        "    name: Step 0\n",
        "    goal: Do first thing\n",
        "    verify:\n",
        "      - Check output\n",
        "    transitions:\n",
        "      - action: goto\n",
        "        target_step: 1\n",
        "  - id: 1\n",
        "    name: Step 1\n",
        "    goal: Do second thing\n",
        "    verify:\n",
        "      - Done\n",
        "    transitions:\n",
        "      - action: complete\n",
    );
    write_skill_md_with_yaml(dir, workflow_name, yaml);
}

/// Write a single-step workflow SKILL.md with a default complete transition.
fn write_complete_skill_md(dir: &std::path::Path, workflow_name: &str) {
    let yaml = concat!(
        "id: test-wf\n",
        "name: Test WF\n",
        "description: test\n",
        "steps:\n",
        "  - id: 0\n",
        "    name: Step 0\n",
        "    goal: Do thing\n",
        "    verify:\n",
        "      - Done\n",
        "    transitions:\n",
        "      - action: complete\n",
    );
    write_skill_md_with_yaml(dir, workflow_name, yaml);
}

/// Write a single-step workflow SKILL.md with a default reexecute transition.
fn write_reexecute_skill_md(dir: &std::path::Path, workflow_name: &str) {
    let yaml = concat!(
        "id: test-wf\n",
        "name: Test WF\n",
        "description: test\n",
        "steps:\n",
        "  - id: 0\n",
        "    name: Step 0\n",
        "    goal: Do thing\n",
        "    verify:\n",
        "      - Done\n",
        "    transitions:\n",
        "      - action: reexecute\n",
        "        target_step: 0\n",
    );
    write_skill_md_with_yaml(dir, workflow_name, yaml);
}

/// Create a session with a workflow run set to the given phase.
/// Caller writes the skill MD first, then calls this.
fn make_session_with_phase(
    tmp: &tempfile::TempDir,
    wf_name: &str,
    phase: Phase,
) -> ConversationSession {
    let mut session = ConversationSession::new(
        "sid".to_string(),
        "model".to_string(),
        tmp.path().to_path_buf(),
    );
    session.set_workflow_port(test_port());
    let mut run = make_test_run(wf_name);
    run.phase = phase;
    session.set_workflow_run(Some(run));
    session.ensure_workflow_handler();
    session
}

/// Collect workflow messages as text strings.
fn wf_messages(session: &ConversationSession) -> Vec<String> {
    session
        .messages
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
        .collect()
}

/// Inject an assistant tool_use + user tool_result pair.
fn inject_tool_exchange(
    session: &mut ConversationSession,
    tc_id: &str,
    tool_name: &str,
    input: &str,
    result: &str,
) {
    session.push_message(
        "assistant",
        vec![ContentBlock::ToolUse {
            id: tc_id.to_string(),
            name: tool_name.to_string(),
            input: input.to_string(),
        }],
    );
    session.push_message(
        "user",
        vec![ContentBlock::ToolResult {
            tool_call_id: tc_id.to_string(),
            content: result.to_string(),
        }],
    );
}

// ── process_workflow_tool_results with ensure ────────────────

#[test]
fn test_process_workflow_tool_results_with_existing_handler() {
    let definition = Workflow {
        id: "test-wf".to_string(),
        name: "Test WF".to_string(),
        description: "test".to_string(),
        definition_path: None,
        version: None,
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![Step {
            id: 0,
            name: "S0".to_string(),
            goal: "g".to_string(),
            verify: vec![],
            jump: vec![],
            transitions: vec![],
            allow_blocked: Some(true),
        }],
    };
    let run = WorkflowRun {
        workflow_id: "wf".to_string(),
        definition_name: "Test WF".to_string(),
        definition_version: "0.1".to_string(),
        current_step: 0,
        phase: Phase::Executing,
        current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::default(),
        pending_verify: PendingVerify::default(),
        paused_reason: String::new(),
    };
    let mut handler = WorkflowHandler::new(
        serde_json::to_value(&run).unwrap(),
        serde_json::to_value(&definition).unwrap(),
        test_port(),
    );

    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "c1".to_string(),
        content: r#"{"action": "workflow_blocked", "reason": "test"}"#.to_string(),
    }];
    assert!(handler.process_content_blocks(&blocks).0);
    assert_eq!(typed_run_value(handler.run_state()).phase, Phase::Blocked);
}

#[test]
fn test_process_workflow_tool_results_via_session() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill_md(tmp.path(), "Test Workflow");

    let mut session = ConversationSession::new(
        "sid".to_string(),
        "model".to_string(),
        tmp.path().to_path_buf(),
    );
    session.set_workflow_port(test_port());
    session.set_workflow_run(Some(make_test_run("Test Workflow")));

    // Build handler first.
    session.ensure_workflow_handler();
    assert!(session.workflow_handler().is_some());

    // Process blocks through handler directly.
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "c1".to_string(),
        content: r#"{"action": "workflow_blocked", "reason": "test"}"#.to_string(),
    }];
    let (processed, _) = session
        .workflow_handler_mut()
        .unwrap()
        .process_content_blocks(&blocks);
    assert!(processed, "direct handler call should work");
    assert_eq!(typed_run(&session).phase, Phase::Blocked);
}

// ── Step 1.3: jump → goal injection / complete exit ─────────

/// After a goto jump, the new step's goal message is injected
/// into the transcript with Normal hint.
#[test]
fn test_goto_jump_injects_goal_message() {
    let tmp = tempfile::tempdir().unwrap();
    write_two_step_skill_md(tmp.path(), "Test WF");
    let mut session = make_session_with_phase(&tmp, "Test WF", Phase::Jumping);

    session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
    session.push_message(
        "assistant",
        vec![ContentBlock::ToolUse {
            id: "tc1".to_string(),
            name: "workflow_jump".to_string(),
            input: r#"{"answers": {}}"#.to_string(),
        }],
    );
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc1".to_string(),
        content: r#"{"action": "workflow_jump", "answers": {}}"#.to_string(),
    }];
    session.process_workflow_tool_results(&blocks);

    let wf = wf_messages(&session);
    assert_eq!(wf.len(), 1, "should have one goal message");
    assert!(wf[0].starts_with("[workflow goal]"), "goal: {}", wf[0]);
    assert!(wf[0].contains("Step 1"), "step 1: {}", wf[0]);
    // pending_goal_hint should be consumed (reset to Normal).
    let run = typed_run(&session);
    assert_eq!(run.pending_goal_hint, GoalHint::Normal);
    assert_eq!(run.current_step, 1);
}

/// After a reexecute jump, the goal message includes the reexecute hint.
#[test]
fn test_reexecute_jump_injects_goal_with_hint() {
    let tmp = tempfile::tempdir().unwrap();
    write_reexecute_skill_md(tmp.path(), "Test WF");
    let mut session = make_session_with_phase(&tmp, "Test WF", Phase::Jumping);

    session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
    session.push_message(
        "assistant",
        vec![ContentBlock::ToolUse {
            id: "tc1".to_string(),
            name: "workflow_jump".to_string(),
            input: r#"{"answers": {}}"#.to_string(),
        }],
    );
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc1".to_string(),
        content: r#"{"action": "workflow_jump", "answers": {}}"#.to_string(),
    }];
    session.process_workflow_tool_results(&blocks);

    let wf = wf_messages(&session);
    assert_eq!(wf.len(), 1);
    assert!(wf[0].starts_with("[workflow goal]"));
    assert!(wf[0].contains("重新执行"), "reexecute hint: {}", wf[0]);
    let run = typed_run(&session);
    assert_eq!(run.pending_goal_hint, GoalHint::Normal);
    assert_eq!(run.current_step, 0);
}

/// After a complete jump, workflow_run is set and handler is cleared.
#[tokio::test]
async fn test_complete_jump_triggers_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    write_complete_skill_md(tmp.path(), "Test WF");

    let mut session = ConversationSession::new(
        "sid".to_string(),
        "model".to_string(),
        tmp.path().to_path_buf(),
    );
    session.set_workflow_port(test_port());
    let mut run = make_test_run("Test WF");
    run.phase = Phase::Jumping;
    session.set_workflow_run(Some(run));
    session.ensure_workflow_handler();

    session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
    session.push_message(
        "assistant",
        vec![ContentBlock::ToolUse {
            id: "tc1".to_string(),
            name: "workflow_jump".to_string(),
            input: r#"{"answers": {}}"#.to_string(),
        }],
    );
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc1".to_string(),
        content: r#"{"action": "workflow_jump", "answers": {}}"#.to_string(),
    }];

    session.process_workflow_tool_results(&blocks);

    // Jump messages removed, no new goal message.
    let wf_roles: Vec<&str> = session
        .messages
        .iter()
        .filter(|m| m.role == "workflow")
        .map(|_m| "workflow")
        .collect();
    assert!(
        wf_roles.is_empty(),
        "no workflow messages should remain after complete"
    );
    // workflow_run should be set (from handler state).
    assert!(session.workflow_run().is_some());
    assert_eq!(session.workflow_run().unwrap().phase, Phase::Complete);
}

/// Write a two-step workflow SKILL.md with jump question and goto transition.
/// Step 0 has a jump question ("go_next"); when answered "yes" → goto step 1.
fn write_two_step_with_jump_skill_md(dir: &std::path::Path, workflow_name: &str) {
    let yaml = concat!(
        "id: test-wf\n",
        "name: Test WF\n",
        "description: test\n",
        "steps:\n",
        "  - id: 0\n",
        "    name: Step 0\n",
        "    goal: Do first thing\n",
        "    verify:\n",
        "      - Check output\n",
        "    jump:\n",
        "      - id: go_next\n",
        "        prompt: Proceed to step 1?\n",
        "        type: boolean\n",
        "    transitions:\n",
        "      - when:\n",
        "          go_next: true\n",
        "        action: goto\n",
        "        target_step: 1\n",
        "      - action: complete\n",
        "  - id: 1\n",
        "    name: Step 1\n",
        "    goal: Do second thing\n",
        "    verify:\n",
        "      - Done\n",
        "    transitions:\n",
        "      - action: complete\n",
    );
    write_skill_md_with_yaml(dir, workflow_name, yaml);
}

/// Full chain: verify → jump → goto → new goal injection.
/// Verifies the complete end-to-end flow:
/// 1. Verify tool result transitions to Jumping, injects jump message.
/// 2. Jump tool result (goto) transitions to Executing, injects goal
///    for new step.
#[test]
fn test_full_chain_verify_jump_goto_goal() {
    let tmp = tempfile::tempdir().unwrap();
    write_two_step_with_jump_skill_md(tmp.path(), "Test WF");
    let mut session = make_session_with_phase(&tmp, "Test WF", Phase::Verifying);

    // Verify → transitions to Jumping.
    session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
    session.process_workflow_tool_results(&[ContentBlock::ToolResult {
        tool_call_id: "tc_verify".to_string(),
        content: r#"{"action": "workflow_verify"}"#.to_string(),
    }]);
    let wf = wf_messages(&session);
    assert_eq!(wf.len(), 1, "jump message should be injected");
    assert!(wf[0].starts_with("Jump"), "should be a jump: {}", wf[0]);

    // Jump (goto step 1) → goal injected.
    session.push_message(
        "assistant",
        vec![ContentBlock::ToolUse {
            id: "tc_jump".to_string(),
            name: "workflow_jump".to_string(),
            input: r#"{"answers": {"go_next": true}}"#.to_string(),
        }],
    );
    session.process_workflow_tool_results(&[ContentBlock::ToolResult {
        tool_call_id: "tc_jump".to_string(),
        content: r#"{"action": "workflow_jump", "answers": {"go_next": true}}"#.to_string(),
    }]);

    let wf = wf_messages(&session);
    assert_eq!(wf.len(), 1, "should have one goal message");
    assert!(wf[0].starts_with("[workflow goal]"), "goal: {}", wf[0]);
    assert!(wf[0].contains("Step 1"), "step 1: {}", wf[0]);

    let run = typed_run(&session);
    assert_eq!(run.phase, Phase::Executing);
    assert_eq!(run.current_step, 1);
    assert_eq!(run.pending_goal_hint, GoalHint::Normal);
}

/// When jump answers match no transition, no goal is injected.
#[tokio::test]
async fn test_jump_no_match_does_not_inject_goal() {
    let tmp = tempfile::tempdir().unwrap();
    write_two_step_with_jump_skill_md(tmp.path(), "Test WF");

    let mut session = ConversationSession::new(
        "sid".to_string(),
        "model".to_string(),
        tmp.path().to_path_buf(),
    );
    session.set_workflow_port(test_port());
    let mut run = make_test_run("Test WF");
    run.phase = Phase::Jumping;
    run.current_step = 0;
    session.set_workflow_run(Some(run));
    session.ensure_workflow_handler();

    session.inject_workflow_message("Jump Step 0 (Step 0):\nProceed to step 1?");
    session.push_message(
        "assistant",
        vec![ContentBlock::ToolUse {
            id: "tc1".to_string(),
            name: "workflow_jump".to_string(),
            input: r#"{"answers": {"go_next": "no"}}"#.to_string(),
        }],
    );

    // Answer "no" doesn't match the goto transition (expects "yes").
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc1".to_string(),
        content: r#"{"action": "workflow_jump", "answers": {"go_next": "no"}}"#.to_string(),
    }];
    session.process_workflow_tool_results(&blocks);

    // No goal message should be injected.
    let wf_goals: Vec<&str> = session
        .messages
        .iter()
        .filter(|m| m.role == "workflow")
        .filter(|m| {
            m.content_blocks.iter().any(|b| match b {
                ContentBlock::Text(t) => t.contains("[workflow goal]"),
                _ => false,
            })
        })
        .map(|_| "goal")
        .collect();
    assert!(
        wf_goals.is_empty(),
        "no goal message should be injected when no transition matches"
    );
}

/// Verify tool exchange is cleaned up when verify transitions to Jumping.
/// The assistant's workflow_verify tool_call and its tool_result are
/// removed, while non-workflow tool calls are preserved.
#[test]
fn test_verify_tool_exchange_cleanup_preserves_non_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    write_two_step_with_jump_skill_md(tmp.path(), "Test WF");
    let mut session = make_session_with_phase(&tmp, "Test WF", Phase::Verifying);

    session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
    inject_tool_exchange(
        &mut session,
        "tc_v",
        "workflow_verify",
        r#"{"result": "pass"}"#,
        r#"{"action": "workflow_verify"}"#,
    );
    inject_tool_exchange(
        &mut session,
        "tc_read",
        "read_file",
        &format!(
            r#"{{"path": "{}"}}"#,
            std::env::temp_dir().join("x").display()
        ),
        "file data",
    );

    session.process_workflow_tool_results(&[ContentBlock::ToolResult {
        tool_call_id: "tc_v".to_string(),
        content: r#"{"action": "workflow_verify"}"#.to_string(),
    }]);

    assert_eq!(typed_run(&session).phase, Phase::Jumping);
    let wf = wf_messages(&session);
    assert_eq!(wf.len(), 1, "jump injected");
    assert!(wf[0].starts_with("Jump"), "jump: {}", wf[0]);

    let read_tools: Vec<&str> = session
        .messages
        .iter()
        .flat_map(|m| &m.content_blocks)
        .filter_map(|b| match b {
            ContentBlock::ToolUse { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert!(read_tools.contains(&"read_file"));
}

/// After goal injection via pending_goal_hint, hint resets to Normal.
#[test]
fn test_pending_goal_hint_resets_after_injection() {
    let tmp = tempfile::tempdir().unwrap();
    write_reexecute_skill_md(tmp.path(), "Test WF");

    let mut session = ConversationSession::new(
        "sid".to_string(),
        "model".to_string(),
        tmp.path().to_path_buf(),
    );
    session.set_workflow_port(test_port());
    let mut run = make_test_run("Test WF");
    run.phase = Phase::Jumping;
    run.pending_goal_hint = GoalHint::Reexecute; // Set hint before jump
    session.set_workflow_run(Some(run));
    session.ensure_workflow_handler();

    session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
    session.push_message(
        "assistant",
        vec![ContentBlock::ToolUse {
            id: "tc1".to_string(),
            name: "workflow_jump".to_string(),
            input: r#"{"answers": {}}"#.to_string(),
        }],
    );
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc1".to_string(),
        content: r#"{"action": "workflow_jump", "answers": {}}"#.to_string(),
    }];

    session.process_workflow_tool_results(&blocks);

    // Handler hint should be Normal after injection consumed it.
    assert_eq!(typed_run(&session).pending_goal_hint, GoalHint::Normal);
}

/// Blocked phase erases verify injection + tool_call + tool_result.
/// Reproduces the design-doc requirement:
/// > Agent 调用 workflow_verify **或 workflow_blocked** 后，Engine 抹除
/// > verify 注入消息 + tool_call + tool_result 三条消息
#[test]
fn test_blocked_triggers_message_erasure() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill_md(tmp.path(), "Test Workflow");
    let mut session = make_session_with_phase(&tmp, "Test Workflow", Phase::Executing);

    // 1) Inject verify message (role: workflow).
    session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
    assert_eq!(wf_messages(&session).len(), 1, "verify injected");

    // 2) Inject assistant tool_call(workflow_blocked) into session.
    session.push_message(
        "assistant",
        vec![ContentBlock::ToolUse {
            id: "tc_blocked".to_string(),
            name: "workflow_blocked".to_string(),
            input: r#"{"reason": "cannot proceed"}"#.to_string(),
        }],
    );

    // 3) Process tool_result (not yet in session messages).
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc_blocked".to_string(),
        content: r#"{"action": "workflow_blocked", "reason": "cannot proceed"}"#.to_string(),
    }];
    let processed = session.process_workflow_tool_results(&blocks);

    assert!(processed, "should have processed blocked action");
    assert_eq!(
        typed_run(&session).phase,
        Phase::Blocked,
        "phase should be Blocked"
    );

    // All three workflow messages erased:
    // - verify injection (role: workflow)
    // - assistant tool_call(workflow_blocked)
    // - user tool_result
    assert_eq!(
        wf_messages(&session).len(),
        0,
        "verify injection should be erased"
    );
    let has_tool_call = session.messages.iter().any(|m| {
        m.content_blocks.iter().any(|b| match b {
            ContentBlock::ToolUse { name, .. } => name == "workflow_blocked",
            _ => false,
        })
    });
    assert!(!has_tool_call, "tool_call should be erased");
    let has_tool_result = session.messages.iter().any(|m| {
        m.content_blocks.iter().any(|b| match b {
            ContentBlock::ToolResult { tool_call_id, .. } => tool_call_id == "tc_blocked",
            _ => false,
        })
    });
    assert!(!has_tool_result, "tool_result should be erased");
}

// ── Step 1.4: reject workflow_start when active workflow exists ────

/// When a workflow_start tool result arrives and there is already an
/// active workflow run, the start is rejected and an error message is
/// injected into the session.
#[test]
fn test_workflow_start_rejected_when_active_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill_md(tmp.path(), "Test Workflow");
    let mut session = make_session_with_phase(&tmp, "Test Workflow", Phase::Executing);

    // Simulate a workflow_start tool result from the LLM.
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc_start".to_string(),
        content: r#"{"action": "workflow_start"}"#.to_string(),
    }];
    let processed = session.process_workflow_tool_results(&blocks);

    // Should not be processed.
    assert!(!processed, "workflow_start should be rejected");
    // Error message injected.
    let wf = wf_messages(&session);
    assert_eq!(wf.len(), 1, "should have one error message");
    assert!(wf[0].contains("无法启动新工作流"), "error: {}", wf[0]);
    // Handler should still have the original run (not overwritten).
    assert_eq!(typed_run(&session).phase, Phase::Executing);
}

/// workflow_start is rejected during Blocked phase.
#[test]
fn test_workflow_start_rejected_when_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill_md(tmp.path(), "Test Workflow");
    let mut session = make_session_with_phase(&tmp, "Test Workflow", Phase::Blocked);

    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc_start".to_string(),
        content: r#"{"action": "workflow_start"}"#.to_string(),
    }];
    let processed = session.process_workflow_tool_results(&blocks);

    assert!(!processed, "workflow_start should be rejected when blocked");
    let wf = wf_messages(&session);
    assert_eq!(wf.len(), 1);
    assert!(wf[0].contains("Blocked"), "phase in error: {}", wf[0]);
}

/// Non-workflow_start actions are still processed when active workflow exists.
#[test]
fn test_non_start_actions_still_processed_when_active() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill_md(tmp.path(), "Test Workflow");
    let mut session = make_session_with_phase(&tmp, "Test Workflow", Phase::Executing);

    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc_blocked".to_string(),
        content: r#"{"action": "workflow_blocked", "reason": "test"}"#.to_string(),
    }];
    let processed = session.process_workflow_tool_results(&blocks);

    assert!(processed, "non-start actions should still be processed");
    assert_eq!(typed_run(&session).phase, Phase::Blocked);
}

/// When workflow is Complete, workflow_start is allowed.
#[test]
fn test_workflow_start_allowed_when_complete() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill_md(tmp.path(), "Test Workflow");
    let mut session = make_session_with_phase(&tmp, "Test Workflow", Phase::Complete);

    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "tc_start".to_string(),
        content: r#"{"action": "workflow_start"}"#.to_string(),
    }];
    let processed = session.process_workflow_tool_results(&blocks);

    // Complete phase: start is allowed, processed by handler normally.
    assert!(processed, "workflow_start should be allowed when complete");
    let wf = wf_messages(&session);
    assert!(wf.is_empty(), "no error message when allowed: {:?}", wf);
}
