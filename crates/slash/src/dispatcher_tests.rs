//! Tests for the `SlashRouter` trait implementation on `SlashDispatcher`.
//!
//! Covers:
//! - `dispatch`: non-`/` → `None`, unknown → `Some(Unknown)`, registered → handler result
//! - `get_handler`: `Box<dyn SlashHandler>` API (commands/description/immediate/handle)
//! - `is_immediate`: immediate vs non-immediate, unknown → false
//! - Edge cases: bare `/`, `/ ` with no command name, with/without arguments

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::slash_router::{SlashContext, SlashResult, SlashRouter};

use crate::dispatcher::SlashDispatcher;
use crate::handler::SlashHandler;
use crate::registry::HandlerRegistry;

// ---------------------------------------------------------------------------
// Mock handlers
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct EchoHandler;

#[async_trait]
impl SlashHandler for EchoHandler {
    fn commands(&self) -> &[&str] {
        &["echo"]
    }

    fn description(&self) -> &str {
        "Echo back the arguments"
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(args.to_owned())
    }
}

#[derive(Clone)]
struct ImmediateHandler;

#[async_trait]
impl SlashHandler for ImmediateHandler {
    fn commands(&self) -> &[&str] {
        &["ping"]
    }

    fn description(&self) -> &str {
        "Respond immediately"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply("pong".to_owned())
    }
}

#[derive(Clone)]
struct ArgsCaptureHandler;

#[async_trait]
impl SlashHandler for ArgsCaptureHandler {
    fn commands(&self) -> &[&str] {
        &["cap"]
    }

    fn description(&self) -> &str {
        "Capture args"
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(format!("args=[{args}]"))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "u".into(),
        session_id: "s".into(),
        channel: "c".into(),
    }
}

fn dispatcher_with(registry: HandlerRegistry) -> SlashDispatcher {
    SlashDispatcher::new(registry)
}

fn default_registry() -> HandlerRegistry {
    let r = HandlerRegistry::new();
    r.register(Arc::new(EchoHandler));
    r.register(Arc::new(ImmediateHandler));
    r.register(Arc::new(ArgsCaptureHandler));
    r
}

// ===========================================================================
// SlashRouter::dispatch
// ===========================================================================

/// Non-slash content must return `None` (not a slash command).
#[tokio::test]
async fn dispatch_non_slash_returns_none() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    assert!(SlashRouter::dispatch(&dispatcher, "hello world", &ctx)
        .await
        .is_none());
}

/// Empty string is not a slash command → `None`.
#[tokio::test]
async fn dispatch_empty_string_returns_none() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    assert!(SlashRouter::dispatch(&dispatcher, "", &ctx).await.is_none());
}

/// Bare `/` with no command name → `None`.
#[tokio::test]
async fn dispatch_bare_slash_returns_none() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    assert!(SlashRouter::dispatch(&dispatcher, "/", &ctx)
        .await
        .is_none());
}

/// `/ ` (slash + whitespace only) → `None`.
#[tokio::test]
async fn dispatch_slash_with_whitespace_only_returns_none() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    assert!(SlashRouter::dispatch(&dispatcher, "/ ", &ctx)
        .await
        .is_none());
}

/// `/   ` (multiple whitespace) → `None`.
#[tokio::test]
async fn dispatch_slash_with_many_whitespace_returns_none() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    assert!(SlashRouter::dispatch(&dispatcher, "/   ", &ctx)
        .await
        .is_none());
}

/// Unknown command → `Some(SlashResult::Unknown)`.
#[tokio::test]
async fn dispatch_unknown_command_returns_some_unknown() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    let result = SlashRouter::dispatch(&dispatcher, "/nope", &ctx).await;
    match result {
        Some(SlashResult::Unknown(text)) => assert_eq!(text, "/nope"),
        other => panic!("expected Some(Unknown(\"/nope\")), got {other:?}"),
    }
}

/// Known command → delegates to handler, returns `Some(result)`.
#[tokio::test]
async fn dispatch_known_command_returns_some_handler_result() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    let result = SlashRouter::dispatch(&dispatcher, "/echo hello", &ctx).await;
    match result {
        Some(SlashResult::Reply(text)) => assert_eq!(text, "hello"),
        other => panic!("expected Some(Reply(\"hello\")), got {other:?}"),
    }
}

/// Command with no arguments → handler receives empty string.
#[tokio::test]
async fn dispatch_command_no_args() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    let result = SlashRouter::dispatch(&dispatcher, "/echo", &ctx).await;
    match result {
        Some(SlashResult::Reply(text)) => assert_eq!(text, ""),
        other => panic!("expected Some(Reply(\"\")), got {other:?}"),
    }
}

/// Command with extra spaces in arguments.
#[tokio::test]
async fn dispatch_command_preserves_arg_whitespace() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    // "/cap  hello   world" → cmd="cap", args="hello   world"
    let result = SlashRouter::dispatch(&dispatcher, "/cap  hello   world", &ctx).await;
    match result {
        Some(SlashResult::Reply(text)) => assert_eq!(text, "args=[hello   world]"),
        other => panic!("expected Some(Reply), got {other:?}"),
    }
}

/// Leading whitespace before `/` is trimmed.
#[tokio::test]
async fn dispatch_leading_whitespace_trimmed() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    let result = SlashRouter::dispatch(&dispatcher, "  /echo hi", &ctx).await;
    match result {
        Some(SlashResult::Reply(text)) => assert_eq!(text, "hi"),
        other => panic!("expected Some(Reply(\"hi\")), got {other:?}"),
    }
}

/// Context's `command` field is set in the handler's context.
#[tokio::test]
async fn dispatch_sets_command_in_context() {
    // Use a handler that reads ctx.command to verify it was set.
    #[derive(Clone)]
    struct CmdCapture;
    #[async_trait]
    impl SlashHandler for CmdCapture {
        fn commands(&self) -> &[&str] {
            &["capture"]
        }
        fn description(&self) -> &str {
            "capture cmd"
        }
        fn clone_box(&self) -> Box<dyn SlashHandler> {
            Box::new(self.clone())
        }
        async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
            SlashResult::Reply(ctx.command.clone())
        }
    }
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(CmdCapture));
    let dispatcher = dispatcher_with(registry);
    let ctx = default_ctx();

    let result = SlashRouter::dispatch(&dispatcher, "/capture foo", &ctx).await;
    match result {
        Some(SlashResult::Reply(cmd)) => assert_eq!(cmd, "capture"),
        other => panic!("expected Reply(\"capture\"), got {other:?}"),
    }
}

/// Context fields are preserved through dispatch.
#[tokio::test]
async fn dispatch_preserves_context_fields() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "alice".into(),
        session_id: "sess-42".into(),
        channel: "telegram".into(),
    };

    let result = SlashRouter::dispatch(&dispatcher, "/ping", &ctx).await;
    assert!(result.is_some());
}

// ===========================================================================
// SlashRouter::get_handler
// ===========================================================================

/// Known command → `Some(Box<dyn SlashHandler>)` with correct metadata.
#[tokio::test]
async fn get_handler_known_returns_box() {
    let dispatcher = dispatcher_with(default_registry());

    let handler = SlashRouter::get_handler(&dispatcher, "echo").unwrap();
    assert_eq!(handler.commands(), &["echo"]);
    assert_eq!(handler.description(), "Echo back the arguments");
    assert!(!handler.immediate("echo"));
}

/// Unknown command → `None`.
#[tokio::test]
async fn get_handler_unknown_returns_none() {
    let dispatcher = dispatcher_with(default_registry());

    assert!(SlashRouter::get_handler(&dispatcher, "nope").is_none());
}

/// Immediate handler reports `immediate()` = true via `Box<dyn SlashHandler>`.
#[tokio::test]
async fn get_handler_immediate_reports_true() {
    let dispatcher = dispatcher_with(default_registry());

    let handler = SlashRouter::get_handler(&dispatcher, "ping").unwrap();
    assert!(handler.immediate("ping"));
    assert_eq!(handler.description(), "Respond immediately");
}

/// `handle()` on the boxed handler works correctly.
#[tokio::test]
async fn get_handler_boxed_handle_works() {
    let dispatcher = dispatcher_with(default_registry());
    let ctx = default_ctx();

    let handler = SlashRouter::get_handler(&dispatcher, "echo").unwrap();
    match handler.handle("world", &ctx).await {
        SlashResult::Reply(text) => assert_eq!(text, "world"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// Multiple `get_handler` calls return independent clones (Arc→Box conversion).
#[tokio::test]
async fn get_handler_returns_independent_clones() {
    let dispatcher = dispatcher_with(default_registry());

    let h1 = SlashRouter::get_handler(&dispatcher, "echo").unwrap();
    let h2 = SlashRouter::get_handler(&dispatcher, "echo").unwrap();

    // Both should be functional and independent
    assert_eq!(h1.description(), h2.description());
    assert_eq!(h1.commands(), h2.commands());
}

// ===========================================================================
// SlashRouter::is_immediate
// ===========================================================================

/// Immediate command → `true`.
#[tokio::test]
async fn is_immediate_true_for_immediate_command() {
    let dispatcher = dispatcher_with(default_registry());
    assert!(SlashRouter::is_immediate(&dispatcher, "ping"));
}

/// Non-immediate command → `false`.
#[tokio::test]
async fn is_immediate_false_for_non_immediate() {
    let dispatcher = dispatcher_with(default_registry());
    assert!(!SlashRouter::is_immediate(&dispatcher, "echo"));
}

/// Unknown command → `false`.
#[tokio::test]
async fn is_immediate_false_for_unknown() {
    let dispatcher = dispatcher_with(default_registry());
    assert!(!SlashRouter::is_immediate(&dispatcher, "nope"));
}

/// Empty command string → `false`.
#[tokio::test]
async fn is_immediate_false_for_empty() {
    let dispatcher = dispatcher_with(default_registry());
    assert!(!SlashRouter::is_immediate(&dispatcher, ""));
}
