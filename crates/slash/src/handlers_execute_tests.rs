//! ExecuteHandler tests for Step 1.4 (error paths, edge cases, state transitions).

use std::collections::HashMap;
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_mode::{parse_execute_args, ExecuteHandler};
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::SlashSessionQuery;
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
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_session::storage::memory::MemoryStorage;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    let storage = Arc::new(MemoryStorage::new());
    let sm = SessionManager::new(&gc, Some(storage), None, ReasoningLevel::default());
    Arc::new(sm)
}

async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "exec-step14-msg-1".to_string(),
        from: "user-exec".to_string(),
        to: "user-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    sm.find_or_create("feishu", &msg, None)
        .await
        .expect("session")
}

async fn create_session_with_plan_mode(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "exec-step14-plan-msg".to_string(),
        from: "user-plan".to_string(),
        to: "user-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("session");

    if let Some(conv) = sm.get_conversation_session(&sid).await {
        conv.write().await.set_session_mode(
            closeclaw_common::SessionMode::Plan,
            closeclaw_session::llm_session::mode_transition::ModeChangeSource::Automatic,
        );
    }

    sid
}

#[tokio::test]
async fn test_execute_plan_mode_empty_args_returns_usage_hint() {
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("请指定要执行的 plan 名称"),
                "should contain usage hint in plan mode, got: {text}"
            );
        }
        other => panic!("expected Reply with usage hint in plan mode, got {other:?}"),
    }
}

// ── Error path tests (Step 1.4) ──────────────────────────────────────────

#[tokio::test]
async fn test_execute_plan_mode_name_not_found() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    fs::write(plans_dir.join("existing-plan.md"), "# Plan\n").unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("nonexistent-plan", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("计划文件解析失败"),
                "should contain parse error message, got: {text}"
            );
            assert!(
                text.contains("nonexistent-plan"),
                "should mention the name that was not found, got: {text}"
            );
        }
        other => panic!("expected Reply for not-found name, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_plan_mode_name_ambiguous() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    fs::write(plans_dir.join("auth-login.md"), "# Plan\n").unwrap();
    fs::write(plans_dir.join("auth-logout.md"), "# Plan\n").unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("auth", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("计划文件解析失败"),
                "should contain parse error message, got: {text}"
            );
        }
        other => panic!("expected Reply for ambiguous name, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_non_plan_mode_name_not_found() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("nonexistent", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("计划文件解析失败"),
                "should contain parse error message, got: {text}"
            );
        }
        other => panic!("expected Reply for not-found in non-plan mode, got {other:?}"),
    }
}

// ── Edge case tests (Step 1.4) ────────────────────────────────────────────

#[tokio::test]
async fn test_execute_empty_string_args_returns_usage_hint() {
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("请指定要执行的 plan 名称"),
                "should contain usage hint, got: {text}"
            );
            assert!(
                text.contains("/execute <plan名称> [附加指令]"),
                "should show usage format, got: {text}"
            );
        }
        other => panic!("expected Reply with usage hint for empty args, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_whitespace_only_instruction_treated_as_none() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    fs::write(plans_dir.join("my-plan.md"), "# Plan\n").unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("my-plan   ", &ctx).await {
        SlashResult::SetMode {
            mode,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(
                initial_input.is_none(),
                "whitespace-only instruction should be None"
            );
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_name_with_md_suffix() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    fs::write(plans_dir.join("my-plan.md"), "# Plan\n").unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("my-plan.md", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            ..
        } => {
            assert_eq!(mode, "auto");
            let path = plan_file_path.unwrap();
            assert!(path.to_string_lossy().ends_with("my-plan.md"));
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_multi_space_preserved_in_instruction() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    fs::write(plans_dir.join("my-plan.md"), "# Plan\n").unwrap();

    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;

    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("my-plan   extra  spaces   ", &ctx).await {
        SlashResult::SetMode {
            mode,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert_eq!(initial_input.as_deref(), Some("extra  spaces"));
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

// ── parse_execute_args tests ────────────────────────────────────────────

#[test]
fn test_parse_execute_args_all_cases() {
    // Empty / whitespace-only → empty name, no instruction
    let (n, _i) = parse_execute_args("");
    assert_eq!(n, "");
    let (n, i) = parse_execute_args("   ");
    assert_eq!(n, "");
    assert!(i.is_none());
    let (n, _i) = parse_execute_args("foo");
    assert_eq!(n, "foo");
    // Name + instruction
    let (n, i) = parse_execute_args("foo bar baz");
    assert_eq!(n, "foo");
    assert_eq!(i.as_deref(), Some("bar baz"));
    // Extra whitespace trimmed around name
    let (n, i) = parse_execute_args("  foo  bar baz  ");
    assert_eq!(n, "foo");
    assert_eq!(i.as_deref(), Some("bar baz"));
    // Whitespace-only instruction → None
    let (n, i) = parse_execute_args("foo   ");
    assert_eq!(n, "foo");
    assert!(i.is_none());
    // Name with .md suffix works
    let (n, i) = parse_execute_args("plan.md instruction");
    assert_eq!(n, "plan.md");
    assert_eq!(i.as_deref(), Some("instruction"));
    // Chinese name + instruction
    let (n, i) = parse_execute_args("修复登录 请优先处理");
    assert_eq!(n, "修复登录");
    assert_eq!(i.as_deref(), Some("请优先处理"));
}

// ── ExecuteHandler with name/instruction tests ─────────────────────────

#[tokio::test]
async fn test_execute_plan_mode_with_name_resolves_plan() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let plan_file = plans_dir.join("my-plan.md");
    fs::write(&plan_file, "# My Plan\n").unwrap();
    let sm = make_session_manager_with_storage();
    let sid = create_session_with_plan_mode(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    // name only
    match h.handle("my-plan", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            reply_message,
            ..
        } => {
            assert_eq!(mode, "auto");
            let path = plan_file_path.unwrap();
            assert!(path.to_string_lossy().ends_with("my-plan.md"));
            assert!(initial_input.is_none());
            assert_eq!(reply_message.as_deref(), Some("开始执行"));
        }
        other => panic!("expected SetMode{{mode: \"auto\", ..}}, got {other:?}"),
    }
    // name + instruction
    match h.handle("my-plan 请优先处理", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_some());
            assert_eq!(initial_input.as_deref(), Some("请优先处理"));
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}

#[tokio::test]
async fn test_execute_non_plan_mode_with_name_and_instruction() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let plans_dir = tmp.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let plan_file = plans_dir.join("my-plan.md");
    fs::write(&plan_file, "# My Plan\n").unwrap();
    let sm = make_session_manager_with_storage();
    let sid = create_test_session(&sm).await;
    sm.set_workdir(&sid, tmp.path().to_path_buf()).await;
    let h = ExecuteHandler::new(Arc::clone(&sm) as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    // name only → plan_file_path set, no instruction
    match h.handle("my-plan", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_some());
            assert!(initial_input.is_none());
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
    // name + instruction → both set
    match h.handle("my-plan 请先完成 lint", &ctx).await {
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            ..
        } => {
            assert_eq!(mode, "auto");
            assert!(plan_file_path.is_some());
            assert_eq!(initial_input.as_deref(), Some("请先完成 lint"));
        }
        other => panic!("expected SetMode, got {other:?}"),
    }
}
