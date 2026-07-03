//! Unit tests for the slash command module.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::dispatcher::parse_slash;
use crate::handler::SlashHandler;
use crate::registry::HandlerRegistry;
use crate::ExecHandler;
use closeclaw_common::slash_router::SlashResult;

// ---------------------------------------------------------------------------
// Mock handlers
// ---------------------------------------------------------------------------

struct EchoHandler;

#[async_trait::async_trait]
impl SlashHandler for EchoHandler {
    fn commands(&self) -> &[&str] {
        &["echo"]
    }

    fn description(&self) -> &str {
        "Echo back the arguments"
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(args.to_owned())
    }
}

struct HelpHandler;

#[async_trait::async_trait]
impl SlashHandler for HelpHandler {
    fn commands(&self) -> &[&str] {
        &["help"]
    }

    fn description(&self) -> &str {
        "Print this help message"
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply("help text".to_owned())
    }
}

struct RiskyHandler;

#[async_trait::async_trait]
impl SlashHandler for RiskyHandler {
    fn commands(&self) -> &[&str] {
        &["exec"]
    }

    fn description(&self) -> &str {
        "Execute a shell command"
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(format!("executed: {args}"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_parse_slash() {
    // Normal: command + args
    let (cmd, args) = parse_slash("/exec ls -la").unwrap();
    assert_eq!(cmd, "exec");
    assert_eq!(args, "ls -la");

    // No args
    let (cmd, args) = parse_slash("/help").unwrap();
    assert_eq!(cmd, "help");
    assert_eq!(args, "");

    // Not a slash command
    assert!(parse_slash("hello").is_none());

    // Bare slash
    assert!(parse_slash("/").is_none());
}

#[tokio::test]
async fn test_handler_registry() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(EchoHandler));
    registry.register(Arc::new(HelpHandler));

    // Hit
    let h = registry.get("echo").unwrap();
    assert_eq!(h.commands(), ["echo"]);

    let h = registry.get("help").unwrap();
    assert_eq!(h.commands(), ["help"]);

    // Miss
    assert!(registry.get("unknown").is_none());
}

#[tokio::test]
async fn test_slash_handler_immediate_default() {
    // Default is false
    let safe = EchoHandler;
    assert!(!safe.immediate("echo"));

    let risky = RiskyHandler;
    assert!(!risky.immediate("exec"));
}

// ── parse_slash edge cases ──────────────────────────────────────────────────

#[tokio::test]
async fn test_parse_slash_multiple_spaces() {
    let (cmd, args) = parse_slash("/exec  ls   -la").unwrap();
    assert_eq!(cmd, "exec");
    assert_eq!(args, "ls   -la");
}

#[tokio::test]
async fn test_parse_slash_leading_whitespace() {
    let (cmd, args) = parse_slash("  /help  foo").unwrap();
    assert_eq!(cmd, "help");
    assert_eq!(args, "foo");
}

#[tokio::test]
async fn test_parse_slash_only_slash_and_whitespace() {
    // "/  " — the command is empty after trimming slash
    assert!(parse_slash("/  ").is_none());
}

#[tokio::test]
async fn test_parse_slash_command_with_no_args() {
    let (cmd, args) = parse_slash("/compact").unwrap();
    assert_eq!(cmd, "compact");
    assert_eq!(args, "");
}

#[tokio::test]
async fn test_parse_slash_not_a_command() {
    assert!(parse_slash("hello world").is_none());
    assert!(parse_slash("").is_none());
    assert!(parse_slash("  ").is_none());
}

// ── SlashDispatcher::from_shared ─────────────────────────────────────────────

#[tokio::test]
async fn test_dispatcher_from_shared() {
    let registry = Arc::new(HandlerRegistry::new());
    registry.register(Arc::new(EchoHandler));
    let dispatcher = crate::dispatcher::SlashDispatcher::from_shared(Arc::clone(&registry));
    let h = dispatcher.get_handler("echo").unwrap();
    assert_eq!(h.commands(), ["echo"]);
}

// ── SlashDispatcher::dispatch ────────────────────────────────────────────────

#[tokio::test]
async fn test_dispatch_valid_command() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(EchoHandler));
    let dispatcher = crate::dispatcher::SlashDispatcher::new(registry);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };
    match dispatcher.dispatch("/echo hello world", &ctx).await {
        SlashResult::Reply(text) => assert_eq!(text, "hello world"),
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_dispatch_unknown_command() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(EchoHandler));
    let dispatcher = crate::dispatcher::SlashDispatcher::new(registry);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };
    match dispatcher.dispatch("/nonexistent", &ctx).await {
        SlashResult::Unknown(text) => assert_eq!(text, "/nonexistent"),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[tokio::test]
async fn test_dispatch_not_a_slash_command() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(EchoHandler));
    let dispatcher = crate::dispatcher::SlashDispatcher::new(registry);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };
    match dispatcher.dispatch("hello world", &ctx).await {
        SlashResult::Unknown(text) => assert_eq!(text, "hello world"),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[tokio::test]
async fn test_dispatch_sets_command_in_context() {
    // The dispatcher sets `ctx.command` to the matched command name.
    // We verify this by using a custom handler that reads it.
    struct CmdCaptureHandler;
    #[async_trait::async_trait]
    impl SlashHandler for CmdCaptureHandler {
        fn commands(&self) -> &[&str] {
            &["capture"]
        }
        fn description(&self) -> &str {
            "capture"
        }
        async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
            SlashResult::Reply(ctx.command.clone())
        }
    }
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(CmdCaptureHandler));
    let dispatcher = crate::dispatcher::SlashDispatcher::new(registry);
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };
    match dispatcher.dispatch("/capture some args", &ctx).await {
        SlashResult::Reply(cmd) => assert_eq!(cmd, "capture"),
        other => panic!("expected Reply with command name, got {other:?}"),
    }
}

// ── HandlerRegistry: multi-command handler and Default ───────────────────────

#[test]
fn test_registry_multi_command_handler() {
    let registry = HandlerRegistry::new();
    // RiskyHandler has command "exec"
    registry.register(Arc::new(RiskyHandler));
    let h = registry.get("exec").unwrap();
    assert_eq!(h.commands(), ["exec"]);
}

#[test]
fn test_registry_default() {
    let registry = HandlerRegistry::default();
    assert_eq!(registry.iter().len(), 0);
}

// ── ExecHandler metadata ────────────────────────────────────────────────────

#[test]
fn test_exec_handler_requires_permission() {
    let h = ExecHandler;
    assert!(h.requires_permission());
}

#[test]
fn test_exec_handler_commands_and_description() {
    let h = ExecHandler;
    assert_eq!(h.commands(), &["exec"]);
    assert_eq!(
        h.description(),
        "以 owner 身份执行 shell 命令（需权限审批）"
    );
    assert!(!h.immediate("exec"));
}

#[tokio::test]
async fn test_exec_handler_handle_returns_exec_result() {
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };
    match ExecHandler.handle("ls -la", &ctx).await {
        SlashResult::Exec { command } => assert_eq!(command, "ls -la"),
        other => panic!("expected Exec, got {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_handler_empty_args() {
    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };
    match ExecHandler.handle("", &ctx).await {
        SlashResult::Exec { command } => assert_eq!(command, ""),
        other => panic!("expected Exec, got {other:?}"),
    }
}
