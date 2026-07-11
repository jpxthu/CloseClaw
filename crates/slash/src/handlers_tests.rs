//! Unit tests for built-in handlers (Step 1.6) and registry/dispatcher helpers.

use std::path::Path;
use std::sync::Arc;

use crate::context::SlashContext;
use crate::dispatcher::SlashDispatcher;
use crate::handler::SlashHandler;
use crate::handlers::{
    ClearHandler, CompactHandler, HelpHandler, ReasoningHandler, SystemHandler, WorkdirHandler,
};
use crate::registry::HandlerRegistry;
use crate::VerboseHandler;
use closeclaw_common::slash_router::{SlashResult, SystemAppendAction};
use closeclaw_common::VerbosityLevel;
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_session::persistence::ReasoningLevel;

// ── Mock handler ────────────────────────────────────────────────────────────

pub(crate) struct MockHandler {
    pub(crate) cmds: Vec<&'static str>,
    pub(crate) desc: &'static str,
    pub(crate) imm: bool,
    pub(crate) reply_text: String,
}

#[async_trait::async_trait]
impl SlashHandler for MockHandler {
    fn commands(&self) -> &[&str] {
        &self.cmds
    }
    fn description(&self) -> &str {
        self.desc
    }
    fn immediate(&self, _cmd: &str) -> bool {
        self.imm
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(self.reply_text.clone())
    }
}

pub(crate) fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

// ── CompactHandler tests ────────────────────────────────────────────────────

#[tokio::test]
async fn test_compact_handler_no_args() {
    let ctx = dummy_ctx();
    let result = CompactHandler.handle("", &ctx).await;
    match result {
        SlashResult::Compact { instruction } => assert_eq!(instruction, None),
        _ => panic!("expected Compact with None instruction"),
    }
}

#[tokio::test]
async fn test_compact_handler_with_instruction() {
    let ctx = dummy_ctx();
    let result = CompactHandler.handle("  保留 API 列表  ", &ctx).await;
    match result {
        SlashResult::Compact { instruction } => {
            assert_eq!(instruction, Some("保留 API 列表".to_owned()));
        }
        _ => panic!("expected Compact with instruction"),
    }
}

#[test]
fn test_compact_handler_commands_and_description() {
    let h = CompactHandler;
    assert_eq!(h.commands(), &["compact"]);
    assert_eq!(h.description(), "手动压缩对话历史");
    assert!(!h.immediate("compact"));
}

// ── ClearHandler tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_clear_handler_handle_returns_reply() {
    use closeclaw_gateway::session_manager::SessionManager;
    use closeclaw_gateway::DmScope;
    use closeclaw_session::bootstrap::loader::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &gc,
        None, // storage
        None, // workspace_dir
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let handler = ClearHandler::new(sm);
    let ctx = dummy_ctx();
    let result = handler.handle("", &ctx).await;
    match result {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("System prompt 缓存已清除"),
                "expected clear message, got: {text}"
            );
        }
        _ => panic!("expected Reply variant"),
    }
}

// ── HelpHandler tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_help_handler_lists_commands() {
    let registry = HandlerRegistry::new();
    let mock = Arc::new(MockHandler {
        cmds: vec!["mock"],
        desc: "a mock command",
        imm: false,
        reply_text: "mock reply".to_owned(),
    });
    registry.register(mock);

    let help = HelpHandler::new(Arc::new(registry));
    let ctx = dummy_ctx();
    let result = help.handle("", &ctx).await;
    match result {
        SlashResult::Reply(text) => {
            // HelpHandler reads from registry; it does not self-register.
            assert!(text.contains("/mock"), "should contain /mock, got: {text}");
            assert!(
                text.contains("a mock command"),
                "should contain description, got: {text}"
            );
        }
        _ => panic!("expected Reply"),
    }
}

#[test]
fn test_help_handler_commands_and_description() {
    let registry = HandlerRegistry::new();
    let help = HelpHandler::new(Arc::new(registry));
    assert_eq!(help.commands(), &["help"]);
    assert_eq!(help.description(), "显示所有可用指令");
    assert!(help.immediate("help"));
}

// ── HandlerRegistry iter / all_commands tests ───────────────────────────────

#[test]
fn test_registry_iter_empty() {
    let registry = HandlerRegistry::new();
    assert_eq!(registry.iter().len(), 0);
}

#[test]
fn test_registry_iter_single() {
    let registry = HandlerRegistry::new();
    let h = Arc::new(MockHandler {
        cmds: vec!["foo"],
        desc: "foo",
        imm: false,
        reply_text: String::new(),
    });
    registry.register(h);
    assert_eq!(registry.iter().len(), 1);
}

#[test]
fn test_registry_iter_multi() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(MockHandler {
        cmds: vec!["a"],
        desc: "a",
        imm: false,
        reply_text: String::new(),
    }));
    registry.register(Arc::new(MockHandler {
        cmds: vec!["b"],
        desc: "b",
        imm: false,
        reply_text: String::new(),
    }));
    assert_eq!(registry.iter().len(), 2);
}

#[test]
fn test_registry_all_commands() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(MockHandler {
        cmds: vec!["alpha"],
        desc: "",
        imm: false,
        reply_text: String::new(),
    }));
    registry.register(Arc::new(MockHandler {
        cmds: vec!["beta"],
        desc: "",
        imm: false,
        reply_text: String::new(),
    }));
    let cmds = registry.all_commands();
    assert_eq!(cmds.len(), 2);
    assert!(cmds.contains(&"alpha".to_owned()));
    assert!(cmds.contains(&"beta".to_owned()));
}

// ── SlashDispatcher is_immediate / all_handlers tests ───────────────────────

#[test]
fn test_dispatcher_is_immediate_true() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(MockHandler {
        cmds: vec!["test_cmd"],
        desc: "",
        imm: true,
        reply_text: String::new(),
    }));
    let dispatcher = SlashDispatcher::new(registry);
    assert!(dispatcher.is_immediate("test_cmd"));
}

#[test]
fn test_dispatcher_is_immediate_false() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(MockHandler {
        cmds: vec!["test_cmd"],
        desc: "",
        imm: false,
        reply_text: String::new(),
    }));
    let dispatcher = SlashDispatcher::new(registry);
    assert!(!dispatcher.is_immediate("test_cmd"));
}

// ── WorkdirHandler tests ────────────────────────────────────────────────────

/// Construct a SessionManager the same way `test_clear_handler_handle_returns_reply`
/// does. Returns just the manager — tests that need a session call
/// `create_test_session` to obtain a `session_id`.
fn make_workdir_session_manager() -> std::sync::Arc<SessionManager> {
    use closeclaw_gateway::DmScope;
    use closeclaw_session::bootstrap::loader::BootstrapMode;
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
        None, // storage
        None, // workspace_dir
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

/// Pre-create a session via `SessionManager::find_or_create` and return its id.
/// The returned id can be used to build a `SlashContext` so the handler resolves
/// to a real session for `get_conversation_session`.
async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "workdir-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    let account_id: Option<&str> = None;
    sm.find_or_create("feishu", &msg, account_id)
        .await
        .expect("session")
}

#[test]
fn test_workdir_handler_commands_and_description() {
    let sm = make_workdir_session_manager();
    let h = WorkdirHandler::new(sm);
    assert_eq!(h.commands(), &["cd", "pwd", "git"]);
    assert_eq!(h.description(), "工作目录操作");
    assert!(!h.immediate("cd"));
}

#[tokio::test]
async fn test_workdir_handler_cd_valid_path() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    ctx.command = "cd".to_owned();
    let target = std::env::temp_dir();
    match h.handle(&target.to_string_lossy(), &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("工作目录已变更为"), "got: {t}"),
        _other => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_workdir_handler_cd_invalid_and_no_args() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm));
    // invalid path
    let mut ctx = dummy_ctx();
    ctx.session_id = sid.clone();
    ctx.command = "cd".to_owned();
    match h.handle("/nonexistent_xyz_path", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("目录不存在"), "got: {t}"),
        _other => panic!("expected Reply"),
    }
    // no args
    let mut ctx2 = dummy_ctx();
    ctx2.session_id = sid;
    ctx2.command = "cd".to_owned();
    match h.handle("", &ctx2).await {
        SlashResult::Reply(t) => assert!(t.contains("用法"), "got: {t}"),
        _other => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_workdir_handler_pwd_and_git() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm));
    // pwd
    let mut ctx = dummy_ctx();
    ctx.session_id = sid.clone();
    ctx.command = "pwd".to_owned();
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert!(!t.is_empty()),
        _ => panic!("expected Reply"),
    }
    // git placeholder
    let mut ctx2 = dummy_ctx();
    ctx2.session_id = sid;
    ctx2.command = "git".to_owned();
    match h.handle("status", &ctx2).await {
        SlashResult::Reply(t) => assert!(t.contains("当前目录不是 git 仓库"), "got: {t}"),
        _ => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_workdir_handler_cd_no_session() {
    let sm = make_workdir_session_manager();
    let h = WorkdirHandler::new(sm);
    let ctx = SlashContext {
        command: "cd".to_owned(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("/tmp", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("当前会话未激活"), "got: {t}"),
        other => panic!("expected Reply with no-session, got {other:?}"),
    }
}

#[tokio::test]
async fn test_workdir_handler_pwd_no_session() {
    let sm = make_workdir_session_manager();
    let h = WorkdirHandler::new(sm);
    let ctx = SlashContext {
        command: "pwd".to_owned(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("当前会话未激活"), "got: {t}"),
        other => panic!("expected Reply with no-session, got {other:?}"),
    }
}

#[tokio::test]
async fn test_workdir_handler_cd_root_path() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    ctx.command = "cd".to_owned();
    match h.handle("/", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("工作目录已变更为"), "got: {t}"),
        _other => panic!("expected Reply"),
    }
}

// ── ReasoningHandler tests ─────────────────────────────────────────────────

#[test]
fn test_reasoning_handler_commands_and_description() {
    let sm = make_workdir_session_manager();
    let h = ReasoningHandler::new(sm);
    assert_eq!(h.commands(), &["reasoning"]);
    assert_eq!(h.description(), "查询或设置推理深度");
    assert!(h.immediate("reasoning"));
}

#[tokio::test]
async fn test_reasoning_handler_no_args_returns_current_level() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = ReasoningHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("high"), "got: {t}"),
        _other => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_reasoning_handler_valid_levels() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = ReasoningHandler::new(Arc::clone(&sm));
    let cases = &[
        ("low", ReasoningLevel::Low),
        ("medium", ReasoningLevel::Medium),
        ("high", ReasoningLevel::High),
        ("max", ReasoningLevel::Max),
        ("off", ReasoningLevel::Low),
    ];
    for (arg, expected) in cases {
        let mut ctx = dummy_ctx();
        ctx.session_id = sid.clone();
        match h.handle(arg, &ctx).await {
            SlashResult::SetReasoning { level } => assert_eq!(level, *expected),
            _other => panic!("expected SetReasoning"),
        }
    }
}

#[tokio::test]
async fn test_reasoning_handler_invalid_level() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = ReasoningHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("banana", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("无效的推理深度"), "got: {t}"),
        _other => panic!("expected Reply error"),
    }
}

#[tokio::test]
async fn test_reasoning_handler_no_session_no_args() {
    let sm = make_workdir_session_manager();
    let h = ReasoningHandler::new(sm);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("当前会话未激活"), "got: {t}"),
        other => panic!("expected Reply with no-session, got {other:?}"),
    }
}

#[tokio::test]
async fn test_reasoning_handler_with_whitespace_args() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = ReasoningHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("  high  ", &ctx).await {
        SlashResult::SetReasoning { level } => {
            assert_eq!(level, closeclaw_session::persistence::ReasoningLevel::High)
        }
        other => panic!("expected SetReasoning, got {other:?}"),
    }
}

// ── SystemHandler tests ─────────────────────────────────────────────────────

#[test]
fn test_system_handler_commands_and_description() {
    let h = SystemHandler::new(make_workdir_session_manager());
    assert_eq!(h.commands(), &["system"]);
    assert_eq!(h.description(), "管理 system prompt 追加区");
    assert!(!h.immediate("system"));
}

#[tokio::test]
async fn test_system_add_returns_append() {
    let h = SystemHandler::new(make_workdir_session_manager());
    let ctx = dummy_ctx();
    match h.handle("add 你好", &ctx).await {
        SlashResult::SystemAppend {
            action: SystemAppendAction::Add(t),
        } => assert_eq!(t, "你好"),
        _ => panic!("expected SystemAppend::Add"),
    }
}

#[tokio::test]
async fn test_system_add_empty_returns_usage() {
    let h = SystemHandler::new(make_workdir_session_manager());
    match h.handle("add", &dummy_ctx()).await {
        SlashResult::Reply(t) => assert!(t.contains("用法"), "got: {t}"),
        _ => panic!("expected Reply with usage"),
    }
}

#[tokio::test]
async fn test_system_clear_returns_clear() {
    let h = SystemHandler::new(make_workdir_session_manager());
    match h.handle("clear", &dummy_ctx()).await {
        SlashResult::SystemAppend {
            action: SystemAppendAction::Clear,
        } => {}
        _ => panic!("expected SystemAppend::Clear"),
    }
}

#[tokio::test]
async fn test_system_unknown_subcommand() {
    let h = SystemHandler::new(make_workdir_session_manager());
    match h.handle("foo", &dummy_ctx()).await {
        SlashResult::Reply(t) => assert!(t.contains("未知子指令"), "got: {t}"),
        _ => panic!("expected Reply with unknown"),
    }
}

#[tokio::test]
async fn test_system_plus_syntax_returns_append() {
    let h = SystemHandler::new(make_workdir_session_manager());
    let ctx = dummy_ctx();
    match h.handle("+ 追加指令", &ctx).await {
        SlashResult::SystemAppend {
            action: SystemAppendAction::Add(t),
        } => assert_eq!(t, "追加指令"),
        _ => panic!("expected SystemAppend::Add for + syntax"),
    }
}

#[tokio::test]
async fn test_system_plus_empty_returns_usage() {
    let h = SystemHandler::new(make_workdir_session_manager());
    match h.handle("+", &dummy_ctx()).await {
        SlashResult::Reply(t) => assert!(t.contains("用法"), "got: {t}"),
        _ => panic!("expected Reply with usage for empty +"),
    }
}

#[tokio::test]
async fn test_system_list_no_session() {
    let h = SystemHandler::new(make_workdir_session_manager());
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("list", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("当前会话未激活"), "got: {t}"),
        other => panic!("expected Reply with no-session, got {other:?}"),
    }
}

#[tokio::test]
async fn test_system_no_args_no_session() {
    let h = SystemHandler::new(make_workdir_session_manager());
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("当前会话未激活"), "got: {t}"),
        other => panic!("expected Reply with no-session, got {other:?}"),
    }
}

#[test]
fn test_system_append_action_match() {
    // Add variant
    match (SlashResult::SystemAppend {
        action: SystemAppendAction::Add("test".to_owned()),
    }) {
        SlashResult::SystemAppend {
            action: SystemAppendAction::Add(s),
        } => assert_eq!(s, "test"),
        _ => panic!("Add match failed"),
    }
    // Clear variant
    match (SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    }) {
        SlashResult::SystemAppend {
            action: SystemAppendAction::Clear,
        } => {}
        _ => panic!("Clear match failed"),
    }
}

// ── VerboseHandler tests ─────────────────────────────────────────────────────

#[test]
fn test_verbose_handler_commands_and_description() {
    let sm = make_workdir_session_manager();
    let h = VerboseHandler::new(sm);
    assert_eq!(h.commands(), &["verbose"]);
    assert_eq!(h.description(), "查询或设置输出详细度");
    assert!(h.immediate("verbose"));
}

#[tokio::test]
async fn test_verbose_query_no_args() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = VerboseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(t.contains("当前输出详细度"), "got: {t}");
            assert!(t.contains("normal"), "default should be normal, got: {t}");
        }
        _ => panic!("expected Reply with current level"),
    }
}

#[tokio::test]
async fn test_verbose_set_valid_levels() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = VerboseHandler::new(Arc::clone(&sm));
    let cases = &[
        ("full", VerbosityLevel::Full),
        ("normal", VerbosityLevel::Normal),
        ("off", VerbosityLevel::Off),
    ];
    for (arg, expected) in cases {
        let mut ctx = dummy_ctx();
        ctx.session_id = sid.clone();
        match h.handle(arg, &ctx).await {
            SlashResult::SetVerbosity { level } => assert_eq!(level, *expected),
            other => panic!("expected SetVerbosity for arg '{arg}', got: {other:?}"),
        }
    }
}

#[tokio::test]
async fn test_verbose_set_invalid_arg() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = VerboseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("banana", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(t.contains("无效的输出详细度"), "got: {t}");
            assert!(t.contains("full"), "should list valid options, got: {t}");
            assert!(t.contains("normal"), "should list valid options, got: {t}");
            assert!(t.contains("off"), "should list valid options, got: {t}");
        }
        other => panic!("expected Reply error, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_verbose_handler_no_session_no_args() {
    let sm = make_workdir_session_manager();
    let h = VerboseHandler::new(sm);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert!(t.contains("当前会话未激活"), "got: {t}"),
        other => panic!("expected Reply with no-session, got {other:?}"),
    }
}

#[tokio::test]
async fn test_verbose_set_with_whitespace_args() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = VerboseHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid;
    match h.handle("  normal  ", &ctx).await {
        SlashResult::SetVerbosity { level } => {
            assert_eq!(level, closeclaw_common::VerbosityLevel::Normal)
        }
        other => panic!("expected SetVerbosity, got {other:?}"),
    }
}

// ── /git status tests ───────────────────────────────────────────────────

/// Helper: set a session's workdir to `path` by directly mutating
/// the ConversationSession via the SessionManager.
async fn set_session_workdir(sm: &Arc<SessionManager>, sid: &str, path: &Path) {
    let conv = sm
        .get_conversation_session(sid)
        .await
        .expect("session should exist");
    let mut cs = conv.write().await;
    cs.set_workdir(path.to_path_buf());
}

#[tokio::test]
async fn test_git_status_in_non_git_repo() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm));
    // Default workdir is /tmp which is not a git repo.
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    match h.handle("status", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(
                t.contains("当前目录不是 git 仓库"),
                "expected non-git message, got: {t}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_git_status_in_git_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_path = tmp.path().join("repo");
    std::fs::create_dir(&repo_path).unwrap();
    // Init git repo with an initial commit so HEAD exists.
    std::process::Command::new("git")
        .args(["init", repo_path.to_str().unwrap()])
        .output()
        .expect("git init failed");
    std::fs::write(repo_path.join(".gitkeep"), "").unwrap();
    std::process::Command::new("git")
        .args(["-C", repo_path.to_str().unwrap(), "add", "."])
        .output()
        .expect("git add failed");
    std::process::Command::new("git")
        .args(["-C", repo_path.to_str().unwrap(), "commit", "-m", "init"])
        .output()
        .expect("git commit failed");

    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    set_session_workdir(&sm, &sid, &repo_path).await;

    let h = WorkdirHandler::new(Arc::clone(&sm));
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    match h.handle("status", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(t.contains("On branch"), "expected branch info, got: {t}");
            assert!(t.contains("status:"), "expected status field, got: {t}");
        }
        other => panic!("expected Reply with branch info, got {other:?}"),
    }
}

#[tokio::test]
async fn test_git_no_args_defaults_to_status() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm));
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    // Empty args should route to status (which returns non-git message).
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(
                t.contains("当前目录不是 git 仓库"),
                "expected non-git message (default to status), got: {t}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_git_unknown_subcommand() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm));
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    match h.handle("unknown", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(
                t.contains("未知子指令"),
                "expected unknown subcommand, got: {t}"
            );
            assert!(
                t.contains("unknown"),
                "should echo the bad subcommand, got: {t}"
            );
        }
        other => panic!("expected Reply with unknown, got {other:?}"),
    }
}

#[tokio::test]
async fn test_git_status_extra_args_ignored() {
    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    let h = WorkdirHandler::new(Arc::clone(&sm));
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    // Extra arguments after 'status' should be ignored.
    match h.handle("status --porcelain", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(
                t.contains("当前目录不是 git 仓库"),
                "expected status output (extra args ignored), got: {t}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_git_status_no_session() {
    let sm = make_workdir_session_manager();
    let h = WorkdirHandler::new(sm);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: "nonexistent".to_owned(),
        channel: "c".to_owned(),
    };
    match h.handle("status", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(t.contains("当前会话未激活"), "got: {t}");
        }
        other => panic!("expected Reply with no-session, got {other:?}"),
    }
}

#[tokio::test]
async fn test_git_status_in_git_repo_with_uncommitted_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_path = tmp.path().join("repo");
    std::fs::create_dir(&repo_path).unwrap();
    std::process::Command::new("git")
        .args(["init", repo_path.to_str().unwrap()])
        .output()
        .expect("git init failed");
    // Initial commit so HEAD exists.
    std::fs::write(repo_path.join(".gitkeep"), "").unwrap();
    std::process::Command::new("git")
        .args(["-C", repo_path.to_str().unwrap(), "add", "."])
        .output()
        .expect("git add failed");
    std::process::Command::new("git")
        .args(["-C", repo_path.to_str().unwrap(), "commit", "-m", "init"])
        .output()
        .expect("git commit failed");
    // Create an untracked file so there are uncommitted changes.
    std::fs::write(repo_path.join("new_file.txt"), "hello").unwrap();

    let sm = make_workdir_session_manager();
    let sid = create_test_session(&sm).await;
    set_session_workdir(&sm, &sid, &repo_path).await;

    let h = WorkdirHandler::new(Arc::clone(&sm));
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: sid,
        channel: "c".to_owned(),
    };
    match h.handle("status", &ctx).await {
        SlashResult::Reply(t) => {
            assert!(t.contains("On branch"), "expected branch info, got: {t}");
            assert!(
                t.contains("uncommitted"),
                "expected uncommitted changes, got: {t}"
            );
        }
        other => panic!("expected Reply with changes, got {other:?}"),
    }
}
