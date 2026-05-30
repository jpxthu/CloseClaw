//! Unit tests for the slash command module.

use std::sync::Arc;

use crate::slash::context::SlashContext;
use crate::slash::dispatcher::{parse_slash, SlashDispatcher};
use crate::slash::handler::{SlashHandler, SlashResult};
use crate::slash::registry::HandlerRegistry;

// ---------------------------------------------------------------------------
// Mock handlers
// ---------------------------------------------------------------------------

struct EchoHandler;

#[async_trait::async_trait]
impl SlashHandler for EchoHandler {
    fn name(&self) -> &str {
        "echo"
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(args.to_owned())
    }
}

struct HelpHandler;

#[async_trait::async_trait]
impl SlashHandler for HelpHandler {
    fn name(&self) -> &str {
        "help"
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply("help text".to_owned())
    }
}

/// A handler that overrides `requires_permission` to return `true`.
struct RiskyHandler;

#[async_trait::async_trait]
impl SlashHandler for RiskyHandler {
    fn name(&self) -> &str {
        "exec"
    }

    fn requires_permission(&self) -> bool {
        true
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
    let mut registry = HandlerRegistry::new();
    registry.register(Arc::new(EchoHandler));
    registry.register(Arc::new(HelpHandler));

    // Hit
    let h = registry.get("echo").unwrap();
    assert_eq!(h.name(), "echo");

    let h = registry.get("help").unwrap();
    assert_eq!(h.name(), "help");

    // Miss
    assert!(registry.get("unknown").is_none());
}

#[tokio::test]
async fn test_slash_handler_requires_permission() {
    // Default is false
    let safe = EchoHandler;
    assert!(!safe.requires_permission());

    // Overridden to true
    let risky = RiskyHandler;
    assert!(risky.requires_permission());
}
