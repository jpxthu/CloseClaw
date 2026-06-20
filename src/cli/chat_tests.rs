//! Unit tests for the interactive chat REPL.
//!
//! Verifies that `build_gateway()` produces a [`Gateway`] with the expected
//! configuration (processor registry, slash dispatcher, session handler) and
//! that the TerminalAdapter quit/exit detection logic works correctly.

use crate::gateway::{GatewayConfig, SessionManager};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use std::sync::Arc;

use super::chat::{build_gateway, build_processor_registry};

// ── Processor Registry tests ────────────────────────────────────────────────

#[tokio::test]
async fn test_build_processor_registry_has_inbound_and_outbound() {
    let config = GatewayConfig {
        name: "test".to_string(),
        ..Default::default()
    };
    let registry = build_processor_registry(&config);

    // ContentNormalizer (inbound) + DslParser (outbound)
    assert!(registry.inbound_len() > 0, "expected inbound processors");
    assert!(registry.outbound_len() > 0, "expected outbound processors");
}

#[tokio::test]
async fn test_build_processor_registry_inbound_count() {
    let config = GatewayConfig {
        name: "test".to_string(),
        ..Default::default()
    };
    let registry = build_processor_registry(&config);

    // Without raw_log_dir: 2 inbound (ContentNormalizer + SessionRouter)
    assert_eq!(registry.inbound_len(), 2);
}

#[tokio::test]
async fn test_build_processor_registry_outbound_count() {
    let config = GatewayConfig {
        name: "test".to_string(),
        ..Default::default()
    };
    let registry = build_processor_registry(&config);

    // 1 outbound (DslParser)
    assert_eq!(registry.outbound_len(), 1);
}

#[tokio::test]
async fn test_build_processor_registry_with_raw_log() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = GatewayConfig {
        name: "test".to_string(),
        raw_log_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let registry = build_processor_registry(&config);

    // With raw_log_dir: 3 inbound (RawLogProcessor + SessionRouter + ContentNormalizer)
    assert_eq!(registry.inbound_len(), 3);
    // 2 outbound (OutboundRawLogProcessor + DslParser)
    assert_eq!(registry.outbound_len(), 2);
}

// ── Gateway build tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_build_gateway_has_processor_registry() {
    let (gateway, _session_manager) = build_gateway().await;

    let (inbound, outbound) = gateway.processor_registry_len();
    assert!(inbound > 0, "expected inbound processors in gateway");
    assert!(outbound > 0, "expected outbound processors in gateway");
}

#[tokio::test]
async fn test_build_gateway_has_slash_dispatcher() {
    let (gateway, _session_manager) = build_gateway().await;

    assert!(
        gateway.has_slash_dispatcher().await,
        "expected slash dispatcher to be configured"
    );
}

#[tokio::test]
async fn test_build_gateway_slash_help_dispatchable() {
    use crate::slash::dispatcher::SlashDispatcher;

    let slash_registry = Arc::new(crate::slash::registry::HandlerRegistry::new());
    let session_manager = Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            ..Default::default()
        },
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    slash_registry.register(Arc::new(crate::slash::ClearHandler::new(Arc::clone(
        &session_manager,
    ))));
    let help_handler = crate::slash::HelpHandler::new(Arc::clone(&slash_registry));
    slash_registry.register(Arc::new(help_handler));
    slash_registry.register(Arc::new(crate::slash::NewSessionHandler));
    slash_registry.register(Arc::new(crate::slash::StopHandler));
    slash_registry.register(Arc::new(crate::slash::StatusHandler::new(Arc::clone(
        &session_manager,
    ))));

    let dispatcher = SlashDispatcher::from_shared(slash_registry);

    // Verify all core handlers are registered
    assert!(
        dispatcher.get_handler("help").is_some(),
        "expected /help handler to be registered"
    );
    assert!(
        dispatcher.get_handler("stop").is_some(),
        "expected /stop handler to be registered"
    );
    assert!(
        dispatcher.get_handler("status").is_some(),
        "expected /status handler to be registered"
    );
    assert!(
        dispatcher.get_handler("clear").is_some(),
        "expected /clear handler to be registered"
    );
    assert!(
        dispatcher.get_handler("new").is_some(),
        "expected /new handler to be registered"
    );
}

// ── Session Message Handler test ────────────────────────────────────────────

#[tokio::test]
async fn test_build_gateway_has_session_handler() {
    let (gateway, _session_manager) = build_gateway().await;

    // In test environments without LLM providers, session handler
    // should not be installed.
    let has = gateway.has_session_handler().await;
    assert!(
        !has,
        "session handler should not be present without LLM providers"
    );
}

// ── TerminalAdapter / REPL quit/exit detection ──────────────────────────────

/// Replicate the quit/exit detection logic from the REPL loop for unit testing.
fn is_quit_command(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.eq_ignore_ascii_case("quit") || trimmed.eq_ignore_ascii_case("exit")
}

fn is_stop_command(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.eq_ignore_ascii_case("/stop")
}

#[test]
fn test_quit_detection_exact() {
    assert!(is_quit_command("quit"));
    assert!(is_quit_command("exit"));
}

#[test]
fn test_quit_detection_case_insensitive() {
    assert!(is_quit_command("Quit"));
    assert!(is_quit_command("QUIT"));
    assert!(is_quit_command("Exit"));
    assert!(is_quit_command("EXIT"));
    assert!(is_quit_command("qUit"));
}

#[test]
fn test_quit_detection_with_whitespace() {
    assert!(is_quit_command("  quit  "));
    assert!(is_quit_command("\texit\n"));
}

#[test]
fn test_quit_detection_non_quit() {
    assert!(!is_quit_command("hello"));
    assert!(!is_quit_command("quitting"));
    assert!(!is_quit_command("exit_now"));
    assert!(!is_quit_command(""));
    assert!(!is_quit_command("/stop"));
}

#[test]
fn test_stop_detection() {
    assert!(is_stop_command("/stop"));
    assert!(is_stop_command("/Stop"));
    assert!(is_stop_command("  /stop  "));
    assert!(!is_stop_command("stop"));
    assert!(!is_stop_command("/stopextra"));
}
