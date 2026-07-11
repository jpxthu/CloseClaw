//! Unit tests for PauseHandler.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_mode::PauseHandler;
use closeclaw_common::plan_state::PlanStatus;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_gateway::session_manager::SessionManager;

// ── Helpers ────────────────────────────────────────────────────────────────

fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

fn make_session_manager_with_storage() -> Arc<SessionManager> {
    use closeclaw_gateway::DmScope;
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_session::storage::memory::MemoryStorage;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    let storage = Arc::new(MemoryStorage::new());
    let sm = SessionManager::new(&gc, Some(storage), None, ReasoningLevel::default());
    Arc::new(sm)
}

fn make_session_manager() -> Arc<SessionManager> {
    use closeclaw_gateway::DmScope;
    use closeclaw_session::persistence::ReasoningLevel;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &gc,
        None,
        None,
        ReasoningLevel::default(),
    ))
}

async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "pause-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    sm.find_or_create("feishu", &msg, None)
        .await
        .expect("session")
}

async fn create_session_with_auto_mode(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "pause-test-msg-2".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    let sid = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("session");

    if let Some(conv) = sm.get_conversation_session(&sid).await {
        conv.write()
            .await
            .set_session_mode(closeclaw_common::SessionMode::Auto);
    }

    sid
}

async fn create_session_with_plan_mode(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "pause-test-msg-3".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    let sid = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("session");

    if let Some(conv) = sm.get_conversation_session(&sid).await {
        conv.write()
            .await
            .set_session_mode(closeclaw_common::SessionMode::Plan);
    }

    sid
}

// ── PauseHandler tests ──────────────────────────────────────────────────

#[test]
fn test_pause_handler_commands_and_description() {
    let sm = make_session_manager();
    let h = PauseHandler::new(sm);
    assert_eq!(h.commands(), &["pause"]);
    assert_eq!(h.description(), "暂停正在执行的 plan");
}

#[test]
fn test_pause_handler_not_immediate() {
    let sm = make_session_manager();
    let h = PauseHandler::new(sm);
    assert!(!h.immediate("pause"));
}

#[tokio::test]
async fn test_pause_handler_no_session() {
    let sm = make_session_manager();
    let h = PauseHandler::new(sm);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("当前会话未激活"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_pause_handler_not_auto_mode() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = PauseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("Auto Mode"),
                "should mention Auto Mode, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_pause_handler_plan_mode_rejected() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    let h = PauseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("Auto Mode"),
                "should mention Auto Mode, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_pause_handler_auto_mode_no_plan() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_auto_mode(&sm).await;
    let h = PauseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("没有活跃的 plan"),
                "should mention no active plan, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_pause_handler_auto_mode_empty_plan_file_path() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_auto_mode(&sm).await;
    sm.set_plan_state(
        &sid,
        closeclaw_common::PlanState {
            phase: closeclaw_common::PlanPhase::FinalPlan,
            status: PlanStatus::Executing,
            plan_file_path: String::new(),
            ..closeclaw_common::PlanState::new()
        },
    )
    .await;

    let h = PauseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("没有关联的 plan 文件"),
                "should mention no plan file, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_pause_handler_auto_mode_executing_plan_success() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | executing |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_auto_mode(&sm).await;
    sm.set_plan_state(
        &sid,
        closeclaw_common::PlanState {
            phase: closeclaw_common::PlanPhase::FinalPlan,
            status: PlanStatus::Executing,
            plan_file_path: plan_file.to_str().unwrap().to_string(),
            ..closeclaw_common::PlanState::new()
        },
    )
    .await;

    let h = PauseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode { mode, .. } => {
            assert_eq!(mode, "plan", "should switch back to plan mode");
        }
        other => panic!("expected SetMode, got {other:?}"),
    }

    let content = fs::read_to_string(&plan_file).unwrap();
    assert!(
        content.contains("| 状态 | paused |"),
        "plan file should show paused status, got: {content}"
    );
}

#[tokio::test]
async fn test_pause_handler_auto_mode_confirmed_plan_success() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | confirmed |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_auto_mode(&sm).await;
    sm.set_plan_state(
        &sid,
        closeclaw_common::PlanState {
            phase: closeclaw_common::PlanPhase::FinalPlan,
            status: PlanStatus::Confirmed,
            plan_file_path: plan_file.to_str().unwrap().to_string(),
            ..closeclaw_common::PlanState::new()
        },
    )
    .await;

    let h = PauseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::SetMode { mode, .. } => {
            assert_eq!(mode, "plan", "should switch back to plan mode");
        }
        other => panic!("expected SetMode, got {other:?}"),
    }

    let content = fs::read_to_string(&plan_file).unwrap();
    assert!(
        content.contains("| 状态 | paused |"),
        "plan file should show paused status, got: {content}"
    );
}

#[tokio::test]
async fn test_pause_handler_already_paused_rejected() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let plan_file = tmp.path().join("test-plan.md");
    fs::write(
        &plan_file,
        "# Test Plan\n\n| 字段 | 值 |\n| 状态 | paused |\n",
    )
    .unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_auto_mode(&sm).await;
    sm.set_plan_state(
        &sid,
        closeclaw_common::PlanState {
            phase: closeclaw_common::PlanPhase::FinalPlan,
            status: PlanStatus::Paused,
            plan_file_path: plan_file.to_str().unwrap().to_string(),
            ..closeclaw_common::PlanState::new()
        },
    )
    .await;

    let h = PauseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("无法暂停 plan"),
                "should reject pausing already paused plan, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}
