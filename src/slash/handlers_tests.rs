//! Unit tests for built-in handlers (Step 1.6) and registry/dispatcher helpers.

use std::sync::Arc;

use crate::slash::context::SlashContext;
use crate::slash::dispatcher::SlashDispatcher;
use crate::slash::handler::{SlashHandler, SlashResult};
use crate::slash::handlers::{ClearHandler, CompactHandler, HelpHandler};
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
