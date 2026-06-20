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
    let (gateway, _session_manager) = build_gateway("test-agent").await;

    let (inbound, outbound) = gateway.processor_registry_len();
    assert!(inbound > 0, "expected inbound processors in gateway");
    assert!(outbound > 0, "expected outbound processors in gateway");
}

#[tokio::test]
async fn test_build_gateway_has_slash_dispatcher() {
    let (gateway, _session_manager) = build_gateway("test-agent").await;

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
    let (gateway, _session_manager) = build_gateway("test-agent").await;

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

// ── Inbound Processor Chain integration tests ─────────────────────────────

use crate::processor_chain::content_normalizer::ContentNormalizer;
use crate::processor_chain::context::ProcessedMessage;
use crate::processor_chain::{MessageContext, ProcessError, ProcessorRegistry};
use async_trait::async_trait;

/// A mock processor that suppresses messages (for testing suppress behavior).
struct SuppressProcessor;

#[async_trait]
impl crate::processor_chain::MessageProcessor for SuppressProcessor {
    fn name(&self) -> &str {
        "suppress-processor"
    }

    fn phase(&self) -> crate::processor_chain::ProcessPhase {
        crate::processor_chain::ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        0
    }

    async fn process(
        &self,
        _ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        Ok(Some(ProcessedMessage {
            content: String::new(),
            metadata: serde_json::Map::new(),
            suppress: true,
            content_blocks: vec![],
        }))
    }
}

/// Build a Gateway with the given ProcessorRegistry (shared across chat tests).
fn make_gw_with_registry(registry: ProcessorRegistry) -> crate::gateway::Gateway {
    let config = GatewayConfig {
        name: "test".to_string(),
        ..Default::default()
    };
    crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::new(crate::gateway::SessionManager::new(
            &crate::gateway::GatewayConfig {
                name: "test".to_string(),
                ..Default::default()
            },
            None,
            None,
            crate::session::bootstrap::BootstrapMode::Full,
            crate::session::persistence::ReasoningLevel::default(),
        )),
        Arc::new(registry),
    )
}

#[tokio::test]
async fn test_process_inbound_chain_cleans_control_characters() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

    let input = "hello\x1b[31mworld\x1b[0m";
    let processed = gateway
        .process_inbound_chain("terminal", "u1", "cli", input, "msg-1")
        .await;

    assert_eq!(processed.content, "helloworld");
    assert!(!processed.suppress);
}

#[tokio::test]
async fn test_process_inbound_chain_suppress_message() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(SuppressProcessor));
    let gateway = make_gw_with_registry(registry);

    let processed = gateway
        .process_inbound_chain("terminal", "u1", "cli", "hello", "msg-1")
        .await;

    assert!(processed.suppress, "expected suppress flag to be set");
}

#[tokio::test]
async fn test_process_inbound_chain_quit_exit_not_affected() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

    for cmd in &["quit", "exit", "/stop"] {
        let processed = gateway
            .process_inbound_chain("terminal", "u1", "cli", cmd, "msg-1")
            .await;
        assert_eq!(processed.content.trim(), *cmd);
    }
}

// ── build_gateway agent_id parameterization tests ─────────────────────────

#[tokio::test]
async fn test_build_gateway_config_name_contains_agent_id() {
    let (gateway, _sm) = build_gateway("my-agent").await;
    assert_eq!(gateway.config_name(), "closeclaw-chat-my-agent");
}

#[tokio::test]
async fn test_build_gateway_different_agent_ids_produce_different_names() {
    let (gw_a, _) = build_gateway("agent-alpha").await;
    let (gw_b, _) = build_gateway("agent-beta").await;
    assert_ne!(gw_a.config_name(), gw_b.config_name());
}

#[tokio::test]
async fn test_build_gateway_agent_id_appears_in_name() {
    let agent_id = "my-special-agent-123";
    let (gateway, _sm) = build_gateway(agent_id).await;
    let name = gateway.config_name();
    assert!(
        name.contains(agent_id),
        "config name '{}' should contain agent_id '{}'",
        name,
        agent_id
    );
}

#[tokio::test]
async fn test_build_gateway_empty_agent_id() {
    let (gateway, _sm) = build_gateway("").await;
    assert_eq!(gateway.config_name(), "closeclaw-chat-");
}

#[tokio::test]
async fn test_build_gateway_agent_id_with_special_characters() {
    let agent_id = "agent/with:special@chars";
    let (gateway, _sm) = build_gateway(agent_id).await;
    assert_eq!(
        gateway.config_name(),
        format!("closeclaw-chat-{}", agent_id)
    );
}

#[tokio::test]
async fn test_build_gateway_agent_id_with_unicode() {
    let agent_id = "agent-\u{4e2d}\u{6587}"; // agent-中文
    let (gateway, _sm) = build_gateway(agent_id).await;
    assert!(
        gateway.config_name().contains(agent_id),
        "config name '{}' should contain unicode agent_id '{}'",
        gateway.config_name(),
        agent_id
    );
}
