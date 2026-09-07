// Unit tests for SlashResultExecutor — covers all SlashResult variant
// execute() behavior. Uses MockSlashEffectExecutor to verify side-effect
// dispatch and MockSessionLookup for session queries.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::executor::{ReplyAction, SideEffectContext, SlashResultExecutor};
use crate::executor_test_utils::{
    make_ctx, ExecutorCall, MockSessionLookup, MockSlashEffectExecutor,
    MockSlashEffectExecutorError,
};
use crate::processor::ContentBlock;
use crate::session_lookup::{PendingMessage, SessionLookup};
use crate::slash_router::{SlashResult, SystemAppendAction};
use crate::{ReasoningLevel, VerbosityLevel};

// ── Test: Reply variant ───────────────────────────────────────────────

#[tokio::test]
async fn test_reply_produces_reply_action_with_correct_text() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s1", "feishu", sm);

    SlashResult::Reply("hello world".into()).execute(&ctx).await;

    let replies = mock.drain_replies();
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0], ContentBlock::Text("hello world".into()));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
    assert!(mock.calls.lock().unwrap().is_empty());
}

// ── Test: Stop variant ────────────────────────────────────────────────

#[tokio::test]
async fn test_stop_calls_execute_stop_with_correct_params() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s2", "feishu", sm);

    SlashResult::Stop {
        cascade: true,
        force: false,
    }
    .execute(&ctx)
    .await;

    let calls = mock.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], ExecutorCall::Stop("s2".into(), true, false));
    drop(calls);

    let replies = mock.drain_replies();
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("已停止当前任务".into()));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ── Test: SetMode with initial_input ──────────────────────────────────

#[tokio::test]
async fn test_set_mode_with_initial_input_sets_mode_and_injects_pending() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let pending = Arc::new(Mutex::new(Vec::<PendingMessage>::new()));
    let sm: Arc<dyn SessionLookup> = Arc::new(MockSessionLookup::with_pending(pending.clone()));
    let ctx = make_ctx(Arc::clone(&mock), "s3", "feishu", sm);

    SlashResult::SetMode {
        mode: "plan".into(),
        plan_file_path: None,
        initial_input: Some("do something".into()),
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SetMode("s3".into(), "plan".into())
    );

    let msgs = pending.lock().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "do something");
    assert_eq!(msgs[0].role.as_deref(), Some("user"));
}

// ── Test: SetMode without initial_input ───────────────────────────────

#[tokio::test]
async fn test_set_mode_without_initial_input_no_pending_message() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let pending = Arc::new(Mutex::new(Vec::<PendingMessage>::new()));
    let sm: Arc<dyn SessionLookup> = Arc::new(MockSessionLookup::with_pending(pending.clone()));
    let ctx = make_ctx(Arc::clone(&mock), "s4", "feishu", sm);

    SlashResult::SetMode {
        mode: "auto".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: Some("已切换到 auto 模式".into()),
    }
    .execute(&ctx)
    .await;

    assert!(pending.lock().unwrap().is_empty());
}

// ── Test: SetMode custom reply_message ────────────────────────────────

#[tokio::test]
async fn test_set_mode_custom_reply_message() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s5", "feishu", sm);

    SlashResult::SetMode {
        mode: "plan".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: Some("自定义回复".into()),
    }
    .execute(&ctx)
    .await;

    let replies = mock.drain_replies();
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("自定义回复".into()));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: SetMode with plan_file_path writes new plan_state ───────────

#[tokio::test]
async fn test_set_mode_with_plan_file_path_writes_new_plan_state() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(crate::PlanState::new());
    // Clear initial state so we test fresh write.
    *plan_handle.lock().unwrap() = None;
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);
    let ctx = make_ctx(Arc::clone(&mock), "s-plan-new", "feishu", sl_ref);

    SlashResult::SetMode {
        mode: "plan".into(),
        plan_file_path: Some(std::path::PathBuf::from("/tmp/plans/my-plan.md")),
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    let stored = plan_handle.lock().unwrap().clone();
    let ps = stored.expect("plan_state should be set");
    assert_eq!(ps.plan_file_path, "/tmp/plans/my-plan.md");
    assert_eq!(ps.phase, crate::PlanPhase::Research);
}

// ── Test: SetMode with plan_file_path updates existing plan_state ─────

#[tokio::test]
async fn test_set_mode_with_plan_file_path_updates_existing_plan_state() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let existing = crate::PlanState {
        phase: crate::PlanPhase::Design,
        plan_file_path: String::new(),
    };
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(existing);
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);
    let ctx = make_ctx(Arc::clone(&mock), "s-plan-upd", "feishu", sl_ref);

    SlashResult::SetMode {
        mode: "plan".into(),
        plan_file_path: Some(std::path::PathBuf::from("/tmp/plans/updated.md")),
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    let ps = plan_handle
        .lock()
        .unwrap()
        .clone()
        .expect("plan_state should be set");
    assert_eq!(ps.plan_file_path, "/tmp/plans/updated.md");
    // Existing phase must be preserved.
    assert_eq!(ps.phase, crate::PlanPhase::Design);
}

// ── Test: SetMode with None plan_file_path does not touch plan_state ──

#[tokio::test]
async fn test_set_mode_with_none_plan_file_path_does_not_touch_plan_state() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(crate::PlanState::new());
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);
    let ctx = make_ctx(Arc::clone(&mock), "s-plan-none", "feishu", sl_ref);

    SlashResult::SetMode {
        mode: "plan".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    // plan_state was not touched — still the initial value (PlanState::new()).
    let stored = plan_handle.lock().unwrap().clone();
    assert!(stored.is_some(), "plan_state should remain untouched");
    assert_eq!(stored.unwrap().plan_file_path, String::new());
}

// ── Test: NewSession variant ──────────────────────────────────────────

#[tokio::test]
async fn test_new_session_creates_session_and_replies() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s6", "telegram", sm);

    SlashResult::NewSession.execute(&ctx).await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::NewSession("s6".into(), "telegram".into())
    );

    let replies = mock.drain_replies();
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("new-session-id")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Compact success ─────────────────────────────────────────────

#[tokio::test]
async fn test_compact_success_replies_with_message() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s7", "feishu", sm);

    SlashResult::Compact {
        instruction: Some("summarize".into()),
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::Compact("s7".into(), Some("summarize".into()))
    );

    let replies = mock.drain_replies();
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("Compacted".into()));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Compact error ───────────────────────────────────────────────

#[tokio::test]
async fn test_compact_error_replies_with_failure_message() {
    // Build a mock that returns an error from execute_compact
    let mock = MockSlashEffectExecutorError;
    let sm = Arc::new(MockSessionLookup::new(None));
    let (tx, mut rx) = mpsc::channel::<ReplyAction>(32);
    let ctx = SideEffectContext {
        session_id: "s8".into(),
        channel: "feishu".into(),
        session_lookup: sm,
        reply_tx: tx,
        executor: Arc::new(mock),
    };

    SlashResult::Compact { instruction: None }
        .execute(&ctx)
        .await;

    let reply = rx.try_recv().unwrap();
    match reply {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.starts_with("Compact failed:")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: SystemAppend Add ────────────────────────────────────────────

#[tokio::test]
async fn test_system_append_add_replies_with_index() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s9", "feishu", sm);

    SlashResult::SystemAppend {
        action: SystemAppendAction::Add("be helpful".into()),
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SystemAppend("s9".into(), SystemAppendAction::Add("be helpful".into()))
    );

    let replies = mock.drain_replies();
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("#1")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: SystemAppend Clear ──────────────────────────────────────────

#[tokio::test]
async fn test_system_append_clear_replies_with_count() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s10", "feishu", sm);

    SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    }
    .execute(&ctx)
    .await;

    let replies = mock.drain_replies();
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("1")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Exec variant ────────────────────────────────────────────────

#[tokio::test]
async fn test_exec_calls_execute_exec_and_replies() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(Some("agent-42".into())));
    let ctx = make_ctx(Arc::clone(&mock), "s11", "feishu", sm);

    SlashResult::Exec {
        command: "ls -la".into(),
        requires_permission: true,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::Exec("s11".into(), "agent-42".into(), "ls -la".into())
    );

    let replies = mock.drain_replies();
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("output: ls -la".into()));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Exec with no chat_id fallback ───────────────────────────────

#[tokio::test]
async fn test_exec_falls_back_to_empty_agent_id() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s12", "feishu", sm);

    SlashResult::Exec {
        command: "pwd".into(),
        requires_permission: false,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::Exec("s12".into(), String::new(), "pwd".into())
    );
}

// ── Test: SetReasoning basic call ─────────────────────────────────────

#[tokio::test]
async fn test_set_reasoning_calls_executor_and_replies() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s13", "feishu", sm);

    SlashResult::SetReasoning {
        level: ReasoningLevel::Max,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SetReasoning("s13".into(), ReasoningLevel::Max)
    );

    let replies = mock.drain_replies();
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("Max")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── SetReasoning: Normal path — no downgrade ──────────────────────────

#[tokio::test]
async fn test_set_reasoning_no_downgrade_reply_high() {
    // High → High (no downgrade).
    let mock = Arc::new(MockSlashEffectExecutor::with_reasoning(Some(
        ReasoningLevel::High,
    )));
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s-rnd", "feishu", sm);
    SlashResult::SetReasoning {
        level: ReasoningLevel::High,
    }
    .execute(&ctx)
    .await;
    let replies = mock.drain_replies();
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => assert!(
            matches!(&blocks[0], ContentBlock::Text(t) if t == "推理深度已设为 High（含 provider 降级后的值）"),
            "got: {:?}",
            &blocks[0],
        ),
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_set_reasoning_no_downgrade_reply_off() {
    // Off → Off (provider supports closing).
    let mock = Arc::new(MockSlashEffectExecutor::with_reasoning(Some(
        ReasoningLevel::Off,
    )));
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s-roff", "feishu", sm);
    SlashResult::SetReasoning {
        level: ReasoningLevel::Off,
    }
    .execute(&ctx)
    .await;
    let replies = mock.drain_replies();
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => assert!(
            matches!(&blocks[0], ContentBlock::Text(t) if t == "推理输出已关闭"),
            "got: {:?}",
            &blocks[0],
        ),
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: SetVerbosity variant ────────────────────────────────────────

#[tokio::test]
async fn test_set_verbosity_calls_executor_and_replies() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s14", "feishu", sm);

    SlashResult::SetVerbosity {
        level: VerbosityLevel::Off,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SetVerbosity("s14".into(), VerbosityLevel::Off)
    );

    let replies = mock.drain_replies();
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("off")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Unknown variant ─────────────────────────────────────────────

#[tokio::test]
async fn test_unknown_command_replies_with_error() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(Arc::clone(&mock), "s16", "feishu", sm);

    SlashResult::Unknown("nonexistent".into())
        .execute(&ctx)
        .await;

    let replies = mock.drain_replies();
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t, "Unknown command: /nonexistent"),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
    assert!(mock.calls.lock().unwrap().is_empty());
}

// ── Test: Plan Mode → Normal clears PlanState ────────────────────────

#[tokio::test]
async fn test_plan_mode_to_normal_clears_plan_state() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let plan = crate::PlanState {
        phase: crate::PlanPhase::Design,
        plan_file_path: "/tmp/plan.md".into(),
    };
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(plan);
    let clear_handle = mock_sl.clear_called_handle();
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);
    let ctx = make_ctx(Arc::clone(&mock), "s-clear-normal", "feishu", sl_ref);
    SlashResult::SetMode {
        mode: "normal".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;
    assert!(
        *clear_handle.lock().unwrap(),
        "clear_plan_state should be called"
    );
    assert!(
        plan_handle.lock().unwrap().is_none(),
        "plan_state should be None after switching to normal"
    );
}

// ── Test: Plan Mode → Auto clears PlanState ──────────────────────────

#[tokio::test]
async fn test_plan_mode_to_auto_clears_plan_state() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let plan = crate::PlanState {
        phase: crate::PlanPhase::Review,
        plan_file_path: "/tmp/plan.md".into(),
    };
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(plan);
    let clear_handle = mock_sl.clear_called_handle();
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);
    let ctx = make_ctx(Arc::clone(&mock), "s-clear-auto", "feishu", sl_ref);

    SlashResult::SetMode {
        mode: "auto".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    assert!(
        *clear_handle.lock().unwrap(),
        "clear_plan_state should be called"
    );
    assert!(
        plan_handle.lock().unwrap().is_none(),
        "plan_state should be None after switching to auto"
    );
}

// ── Test: clear_plan_state idempotent in Normal Mode ─────────────────

#[tokio::test]
async fn test_clear_plan_state_idempotent_in_normal_mode() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    // No plan_state set — already in Normal Mode.
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(crate::PlanState::new());
    // Clear so we start from None.
    *plan_handle.lock().unwrap() = None;
    let clear_handle = mock_sl.clear_called_handle();
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);
    let ctx = make_ctx(Arc::clone(&mock), "s-idempotent", "feishu", sl_ref);

    // First call — no-op, should not panic.
    SlashResult::SetMode {
        mode: "normal".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    assert!(*clear_handle.lock().unwrap());
    assert!(plan_handle.lock().unwrap().is_none());

    // Second call — still idempotent.
    SlashResult::SetMode {
        mode: "normal".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    assert!(*clear_handle.lock().unwrap());
    assert!(plan_handle.lock().unwrap().is_none());
}

// ── Test: Create → Destroy → Re-create PlanState cycle ──────────────

#[tokio::test]
async fn test_plan_state_create_destroy_recreate_cycle() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(crate::PlanState::new());
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);

    // 1. Enter Plan Mode — creates PlanState with plan_file_path.
    {
        let ctx = make_ctx(Arc::clone(&mock), "s-cycle", "feishu", Arc::clone(&sl_ref));
        SlashResult::SetMode {
            mode: "plan".into(),
            plan_file_path: Some(std::path::PathBuf::from("/tmp/plan.md")),
            initial_input: None,
            reply_message: None,
        }
        .execute(&ctx)
        .await;
    }
    {
        let ps = plan_handle.lock().unwrap().clone().expect("should exist");
        assert_eq!(ps.plan_file_path, "/tmp/plan.md");
    }

    // 2. Exit Plan Mode → Normal — PlanState destroyed.
    {
        let ctx = make_ctx(Arc::clone(&mock), "s-cycle", "feishu", Arc::clone(&sl_ref));
        SlashResult::SetMode {
            mode: "normal".into(),
            plan_file_path: None,
            initial_input: None,
            reply_message: None,
        }
        .execute(&ctx)
        .await;
    }
    assert!(
        plan_handle.lock().unwrap().is_none(),
        "plan_state should be None after exit"
    );

    // 3. Re-enter Plan Mode — PlanState re-created.
    {
        let ctx = make_ctx(Arc::clone(&mock), "s-cycle", "feishu", Arc::clone(&sl_ref));
        SlashResult::SetMode {
            mode: "plan".into(),
            plan_file_path: Some(std::path::PathBuf::from("/tmp/plan-v2.md")),
            initial_input: None,
            reply_message: None,
        }
        .execute(&ctx)
        .await;
    }
    {
        let ps = plan_handle.lock().unwrap().clone().expect("should exist");
        assert_eq!(ps.plan_file_path, "/tmp/plan-v2.md");
        assert_eq!(ps.phase, crate::PlanPhase::Research);
    }
}

// ── Test: plan_file_path cleared when switching to non-plan mode ────

/// Verifies that `plan_file_path` (embedded in PlanState) is implicitly
/// cleared when switching to a non-plan mode, since PlanState is set to
/// None by `clear_plan_state`.
#[tokio::test]
async fn test_plan_file_path_cleared_in_non_plan_mode() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let plan = crate::PlanState {
        phase: crate::PlanPhase::Research,
        plan_file_path: "/tmp/plan.md".into(),
    };
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(plan);
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);
    let ctx = make_ctx(Arc::clone(&mock), "s-filepath-clear", "feishu", sl_ref);

    SlashResult::SetMode {
        mode: "normal".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    let stored = plan_handle.lock().unwrap();
    assert!(
        stored.is_none(),
        "plan_state should be None after switching to normal mode"
    );
    // When plan_state is None, plan_file_path is implicitly None too,
    // satisfying the design doc requirement that plan_file_path is
    // cleared on non-plan mode exit.
}

// ── Test: mode=auto + plan_file_path Some does not persist PlanState ──

/// When `mode` is `"auto"` (not `"plan"`), `clear_plan_state` is called
/// immediately after `set_plan_state`, so the final PlanState should be
/// `None` even though `plan_file_path` was provided.
#[tokio::test]
async fn test_set_mode_auto_with_plan_file_path_does_not_persist_plan_state() {
    let mock = Arc::new(MockSlashEffectExecutor::new());
    let (mock_sl, plan_handle) = MockSessionLookup::with_plan_state(crate::PlanState::new());
    let set_calls = mock_sl.set_plan_state_calls_handle();
    let clear_handle = mock_sl.clear_called_handle();
    let sl_ref: Arc<dyn SessionLookup> = Arc::new(mock_sl);
    let ctx = make_ctx(Arc::clone(&mock), "s-auto-no-plan", "feishu", sl_ref);

    SlashResult::SetMode {
        mode: "auto".into(),
        plan_file_path: Some(std::path::PathBuf::from("/tmp/plans/auto-plan.md")),
        initial_input: None,
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    // set_plan_state was called (plan_file_path was provided).
    assert_eq!(*set_calls.lock().unwrap(), 1);
    // clear_plan_state was also called (mode != "plan").
    assert!(*clear_handle.lock().unwrap());
    // Final state: PlanState is None.
    assert!(
        plan_handle.lock().unwrap().is_none(),
        "plan_state should be None because mode=auto triggers clear"
    );
}
