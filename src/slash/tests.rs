//! Unit tests for the slash command module.

use std::sync::Arc;

use crate::slash::context::SlashContext;
use crate::slash::dispatcher::parse_slash;
use crate::slash::handler::{SlashHandler, SlashResult};
use crate::slash::registry::HandlerRegistry;

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
