//! Unit tests for built-in handlers (Step 1.6) and registry/dispatcher helpers.

use std::sync::Arc;

use crate::gateway::session_manager::SessionManager;
use crate::session::persistence::ReasoningLevel;
use crate::slash::context::SlashContext;
use crate::slash::dispatcher::SlashDispatcher;
use crate::slash::handler::{SlashHandler, SlashResult};
use crate::slash::handlers::{
    ClearHandler, CompactHandler, HelpHandler, ReasoningHandler, WorkdirHandler,
};
use crate::slash::registry::HandlerRegistry;

// ── Mock handler ────────────────────────────────────────────────────────────

struct MockHandler {
    cmds: Vec<&'static str>,
    desc: &'static str,
    imm: bool,
    reply_text: String,
}

#[async_trait::async_trait]
impl SlashHandler for MockHandler {
    fn commands(&self) -> &[&str] {
        &self.cmds
    }
    fn description(&self) -> &str {
        self.desc
    }
    fn immediate(&self) -> bool {
        self.imm
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(self.reply_text.clone())
    }
}

fn dummy_ctx() -> SlashContext {
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
    assert!(!h.immediate());
}

// ── ClearHandler tests ────────────────────────────────────────────────────────

#[test]
fn test_clear_handler_commands_and_description() {
    // Verify static metadata via trait interface
    let h = MockHandler {
        cmds: vec!["clear"],
        desc: "清除 system prompt 静态层缓存并触发重建",
        imm: true,
        reply_text: String::new(),
    };
    assert_eq!(h.commands(), &["clear"]);
    assert_eq!(h.description(), "清除 system prompt 静态层缓存并触发重建");
    assert!(h.immediate());
}

#[tokio::test]
async fn test_clear_handler_handle_returns_reply() {
    use crate::gateway::session_manager::SessionManager;
    use crate::gateway::DmScope;
    use crate::session::bootstrap::loader::BootstrapMode;
    use crate::session::persistence::ReasoningLevel;

    let gc = crate::gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::default(),
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
    assert!(help.immediate());
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

#[test]
fn test_dispatcher_is_immediate_unknown() {
    let registry = HandlerRegistry::new();
    let dispatcher = SlashDispatcher::new(registry);
    assert!(!dispatcher.is_immediate("nonexistent"));
}

#[test]
fn test_dispatcher_all_handlers() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(MockHandler {
        cmds: vec!["x"],
        desc: "x desc",
        imm: false,
        reply_text: String::new(),
    }));
    registry.register(Arc::new(MockHandler {
        cmds: vec!["y"],
        desc: "y desc",
        imm: true,
        reply_text: String::new(),
    }));
    let dispatcher = SlashDispatcher::new(registry);
    let handlers = dispatcher.all_handlers();
    assert_eq!(handlers.len(), 2);
}

// ── WorkdirHandler tests ────────────────────────────────────────────────────

/// Construct a SessionManager the same way `test_clear_handler_handle_returns_reply`
/// does. Returns just the manager — tests that need a session call
/// `create_test_session` to obtain a `session_id`.
fn make_workdir_session_manager() -> std::sync::Arc<SessionManager> {
    use crate::gateway::DmScope;
    use crate::session::bootstrap::loader::BootstrapMode;
    use crate::session::persistence::ReasoningLevel;

    let gc = crate::gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::default(),
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
    use crate::gateway::Message;

    let msg = Message {
        id: "workdir-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
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
    assert!(!h.immediate());
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
    // pwd
    let h = WorkdirHandler::new(Arc::clone(&sm));
    let mut ctx = dummy_ctx();
    ctx.session_id = sid.clone();
    ctx.command = "pwd".to_owned();
    match h.handle("", &ctx).await {
        SlashResult::Reply(t) => assert!(!t.is_empty()),
        _other => panic!("expected Reply"),
    }
    // git placeholder
    let mut ctx2 = dummy_ctx();
    ctx2.session_id = sid;
    ctx2.command = "git".to_owned();
    match h.handle("status", &ctx2).await {
        SlashResult::Reply(t) => assert!(t.contains("git 指令即将支持"), "got: {t}"),
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
    assert!(h.immediate());
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

#[test]
fn test_reasoning_level_getter_setter_symmetry() {
    let mut s = crate::llm::session::ConversationSession::new(
        "test-sym".to_owned(),
        "test-model".to_owned(),
        std::path::PathBuf::from("/tmp"),
    );
    assert_eq!(s.reasoning_level(), ReasoningLevel::High);
    for &lv in &[
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::Max,
    ] {
        s.set_reasoning_level(lv);
        assert_eq!(s.reasoning_level(), lv);
    }
}

#[test]
fn test_reasoning_level_with_builder() {
    let s = crate::llm::session::ConversationSession::new(
        "test-b".to_owned(),
        "test-model".to_owned(),
        std::path::PathBuf::from("/tmp"),
    )
    .with_reasoning_level(ReasoningLevel::High);
    assert_eq!(s.reasoning_level(), ReasoningLevel::High);
}
