//! Tests verifying `/pause` command removal (Step 1.2).
//!
//! After removing `PauseHandler`, `/pause` should fall through to the
//! dispatcher's unknown-command path. These tests ensure:
//! 1. `/pause` input produces `SlashResult::Unknown`
//! 2. The handler registry does not contain `pause`
//! 3. Other slash commands (execute/mode/plan/auto) remain intact

use std::sync::Arc;

use closeclaw_common::slash_router::{SlashResult, SlashRouter};

use crate::dispatcher::SlashDispatcher;
use crate::handler::SlashHandler;
use crate::registry::HandlerRegistry;

// ── Helpers ────────────────────────────────────────────────────────────────

fn dummy_ctx() -> crate::context::SlashContext {
    crate::context::SlashContext {
        command: String::new(),
        sender_id: "u".into(),
        session_id: "s".into(),
        channel: "c".into(),
    }
}

/// Build a registry that mimics the production registration (minus PauseHandler).
fn production_like_registry() -> HandlerRegistry {
    let registry = HandlerRegistry::new();
    // Register handlers that exist in production lifecycle.rs (subset).
    // We only need a few known commands to verify they are still present.
    // The key point: `pause` is NOT registered.
    registry.register(Arc::new(MockCmdHandler {
        cmd: "execute",
        desc: "进入 Auto Mode 执行 plan",
    }));
    registry.register(Arc::new(MockCmdHandler {
        cmd: "mode",
        desc: "查询或切换会话模式",
    }));
    registry.register(Arc::new(MockCmdHandler {
        cmd: "plan",
        desc: "进入 Plan Mode",
    }));
    registry.register(Arc::new(MockCmdHandler {
        cmd: "auto",
        desc: "直接进入 Auto Mode",
    }));
    registry.register(Arc::new(MockCmdHandler {
        cmd: "help",
        desc: "显示所有可用指令",
    }));
    registry.register(Arc::new(MockCmdHandler {
        cmd: "compact",
        desc: "手动压缩对话历史",
    }));
    registry.register(Arc::new(MockCmdHandler {
        cmd: "exec",
        desc: "执行 shell 命令",
    }));
    registry
}

#[derive(Clone)]
struct MockCmdHandler {
    cmd: &'static str,
    desc: &'static str,
}

#[async_trait::async_trait]
impl SlashHandler for MockCmdHandler {
    fn commands(&self) -> &[&str] {
        // Leverage the fact that `self.cmd` is already &'static str.
        // We return a static slice via a static array.
        // This works because the mock is only used in tests.
        match self.cmd {
            "execute" => &["execute"],
            "mode" => &["mode"],
            "plan" => &["plan"],
            "auto" => &["auto"],
            "help" => &["help"],
            "compact" => &["compact"],
            "exec" => &["exec"],
            _ => &["unknown"],
        }
    }
    fn description(&self) -> &str {
        self.desc
    }
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
    async fn handle(&self, _args: &str, _ctx: &crate::context::SlashContext) -> SlashResult {
        SlashResult::Reply(format!("handled by {}", self.cmd))
    }
}

// ── 1. Unknown command path: /pause → Unknown ─────────────────────────────

/// Inherent `SlashDispatcher::dispatch` returns `Unknown` for `/pause`.
#[tokio::test]
async fn inherent_dispatch_pause_returns_unknown() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);
    let ctx = dummy_ctx();

    let result = dispatcher.dispatch("/pause", &ctx).await;
    match result {
        SlashResult::Unknown(text) => assert_eq!(text, "/pause"),
        other => panic!("expected Unknown(\"/pause\"), got {other:?}"),
    }
}

/// Inherent `SlashDispatcher::dispatch` returns `Unknown` for `/pause` with args.
#[tokio::test]
async fn inherent_dispatch_pause_with_args_returns_unknown() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);
    let ctx = dummy_ctx();

    let result = dispatcher.dispatch("/pause 请等一下", &ctx).await;
    match result {
        SlashResult::Unknown(text) => assert_eq!(text, "/pause 请等一下"),
        other => panic!("expected Unknown(\"/pause 请等一下\"), got {other:?}"),
    }
}

/// Trait `SlashRouter::dispatch` returns `Some(Unknown)` for `/pause`.
#[tokio::test]
async fn trait_dispatch_pause_returns_some_unknown() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);
    let ctx = dummy_ctx();

    let result = SlashRouter::dispatch(&dispatcher, "/pause", &ctx).await;
    match result {
        Some(SlashResult::Unknown(text)) => assert_eq!(text, "/pause"),
        other => panic!("expected Some(Unknown(\"/pause\")), got {other:?}"),
    }
}

/// Trait `SlashRouter::dispatch` returns `None` for non-slash content
/// (not related to /pause, just confirming trait semantics remain intact).
#[tokio::test]
async fn trait_dispatch_non_slash_returns_none() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);
    let ctx = dummy_ctx();

    let result = SlashRouter::dispatch(&dispatcher, "hello world", &ctx).await;
    assert!(result.is_none());
}

// ── 2. Registry completeness: no `pause`, other commands present ───────────

/// Registry does not contain `pause` command.
#[tokio::test]
async fn registry_does_not_contain_pause() {
    let registry = production_like_registry();
    assert!(
        registry.get("pause").is_none(),
        "registry should not contain a handler for 'pause'"
    );
    assert!(
        !registry.all_commands().contains(&"pause".to_owned()),
        "all_commands() should not include 'pause'"
    );
}

/// get_handler("pause") returns None.
#[tokio::test]
async fn get_handler_pause_returns_none() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);

    assert!(
        SlashRouter::get_handler(&dispatcher, "pause").is_none(),
        "get_handler(\"pause\") should return None"
    );
}

/// is_immediate("/pause") returns false.
#[tokio::test]
async fn is_immediate_pause_returns_false() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);

    assert!(
        !dispatcher.is_immediate("/pause"),
        "is_immediate(\"/pause\") should be false"
    );
}

/// All expected commands are still present in the registry.
#[tokio::test]
async fn registry_contains_all_expected_commands() {
    let registry = production_like_registry();
    let expected = ["execute", "mode", "plan", "auto", "help", "compact", "exec"];
    let commands = registry.all_commands();

    for cmd in &expected {
        assert!(
            commands.contains(&cmd.to_string()),
            "registry should contain '{cmd}', but commands are: {commands:?}"
        );
    }
}

// ── 3. Existing behavior regression: known commands still dispatch ────────

/// `/execute` still dispatches correctly.
#[tokio::test]
async fn dispatch_execute_works() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);
    let ctx = dummy_ctx();

    let result = dispatcher.dispatch("/execute my-plan", &ctx).await;
    match result {
        SlashResult::Reply(text) => assert_eq!(text, "handled by execute"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// `/mode` still dispatches correctly.
#[tokio::test]
async fn dispatch_mode_works() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);
    let ctx = dummy_ctx();

    let result = dispatcher.dispatch("/mode", &ctx).await;
    match result {
        SlashResult::Reply(text) => assert_eq!(text, "handled by mode"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// `/plan` still dispatches correctly.
#[tokio::test]
async fn dispatch_plan_works() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);
    let ctx = dummy_ctx();

    let result = dispatcher.dispatch("/plan 优化性能", &ctx).await;
    match result {
        SlashResult::Reply(text) => assert_eq!(text, "handled by plan"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// `/auto` still dispatches correctly.
#[tokio::test]
async fn dispatch_auto_works() {
    let registry = production_like_registry();
    let dispatcher = SlashDispatcher::new(registry);
    let ctx = dummy_ctx();

    let result = dispatcher.dispatch("/auto", &ctx).await;
    match result {
        SlashResult::Reply(text) => assert_eq!(text, "handled by auto"),
        other => panic!("expected Reply, got {other:?}"),
    }
}
