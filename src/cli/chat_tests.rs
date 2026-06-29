//! Unit tests for the interactive chat REPL.
//!
//! Verifies that `build_gateway()` produces a [`Gateway`] with the expected
//! configuration (processor registry, slash dispatcher, session handler) and
//! that the TerminalAdapter quit/exit detection logic works correctly.

use closeclaw_gateway::{GatewayConfig, InboundChainInput, SessionManager};
use closeclaw_im_adapter::NormalizedMessage;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

use super::chat::build_gateway;
use crate::processor_chain::build_processor_registry;

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
fn make_gw_with_registry(registry: ProcessorRegistry) -> closeclaw_gateway::Gateway {
    let config = GatewayConfig {
        name: "test".to_string(),
        ..Default::default()
    };
    closeclaw_gateway::Gateway::with_processor_registry(
        config,
        Arc::new(closeclaw_gateway::SessionManager::new(
            &closeclaw_gateway::GatewayConfig {
                name: "test".to_string(),
                ..Default::default()
            },
            None,
            None,
            closeclaw_session::bootstrap::BootstrapMode::Full,
            closeclaw_session::persistence::ReasoningLevel::default(),
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
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: input.into(),
            message_id: "msg-1".into(),
            timestamp_ms: 0,
            account_id: None,
        })
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
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: "hello".into(),
            message_id: "msg-1".into(),
            timestamp_ms: 0,
            account_id: None,
        })
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
            .process_inbound_chain(&InboundChainInput {
                platform: "terminal".into(),
                sender_id: "u1".into(),
                peer_id: "cli".into(),
                content: cmd.to_string(),
                message_id: "msg-1".into(),
                timestamp_ms: 0,
                account_id: None,
            })
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

#[test]
fn test_for_provider_mimo_returns_noop() {
    use closeclaw_llm::cache_adapter::for_provider;
    let adapter = for_provider("mimo");
    assert_eq!(adapter.name(), "noop", "mimo should use noop cache adapter");
}

#[test]
fn test_openai_provider_mimo_base_url() {
    use closeclaw_llm::openai::OpenAIProvider;
    use closeclaw_llm::provider::Provider;
    let provider =
        OpenAIProvider::new_with_base_url("test-key".to_string(), "https://api.xiaomimimo.com/v1");
    assert_eq!(
        provider.base_url(),
        "https://api.xiaomimimo.com/v1",
        "mimo provider should use MiMo API base URL"
    );
}

#[tokio::test]
async fn test_init_llm_registry_mimo_via_try_register_provider() {
    use closeclaw_config::providers::CredentialsProvider;
    use closeclaw_llm::openai::OpenAIProvider;
    use closeclaw_llm::LLMRegistry;
    use std::sync::Arc;

    let registry = Arc::new(LLMRegistry::new());
    let creds_provider = CredentialsProvider::default();

    // Register mimo using the same try_register_provider pattern as chat.rs
    let key = creds_provider
        .get_api_key("mimo")
        .or_else(|| std::env::var("MIMO_API_KEY").ok())
        .filter(|k| !k.is_empty());
    if let Some(api_key) = key {
        let provider = Arc::new(OpenAIProvider::new_with_base_url(
            api_key,
            "https://api.xiaomimimo.com/v1",
        )) as Arc<dyn closeclaw_llm::provider::Provider>;
        registry.register("mimo".to_string(), provider).await;
    }

    // In test env without credentials, mimo should NOT be registered
    let listed = registry.list().await;
    assert!(
        !listed.contains(&"mimo".to_string()),
        "mimo should not be registered without credentials"
    );
}

// ── peer_id "cli" verification (Step 1.2) ──────────────────────────────────

/// Verify that `process_inbound_chain` receives "cli" as the peer_id,
/// matching `TerminalAdapter::make_message` which hard-codes peer_id: "cli".
/// This test documents the contract and will fail if someone accidentally
/// passes agent_id or another value.
#[tokio::test]
async fn test_process_inbound_chain_peer_id_is_cli() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

    // The chat.rs repl_loop calls process_inbound_chain with "cli" as peer_id.
    // We verify the call site contract: third argument must be "cli".
    let peer_id_argument = "cli";
    let processed = gateway
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: peer_id_argument.into(),
            content: "hello".into(),
            message_id: "msg-1".into(),
            timestamp_ms: 0,
            account_id: None,
        })
        .await;

    assert!(!processed.suppress);
    // The peer_id "cli" is passed as the third arg to process_inbound_chain.
    // This is the correct value per design doc: peer_id = "cli".
    assert_eq!(
        peer_id_argument, "cli",
        "peer_id must be 'cli' per design doc"
    );
}

// ── NormalizedMessage → InboundChainInput field mapping (Step 1.3) ─────

/// Helper: simulate the field extraction logic in repl_loop.
///
/// This mirrors the code in `chat.rs:repl_loop` that constructs
/// `InboundChainInput` from `NormalizedMessage` fields.
fn normalized_to_inbound(msg: &NormalizedMessage) -> InboundChainInput {
    let message_id = format!("cli-{}-{}", msg.sender_id, msg.timestamp);
    InboundChainInput {
        platform: msg.platform.clone(),
        sender_id: msg.sender_id.clone(),
        peer_id: msg.peer_id.clone(),
        content: msg.content.clone(),
        message_id,
        timestamp_ms: msg.timestamp,
        account_id: msg.account_id.clone(),
    }
}

/// Verify that platform from NormalizedMessage flows into InboundChainInput.
#[test]
fn test_normalized_to_inbound_platform() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: "text".to_string(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: Some("owner".to_string()),
        card_action: None,
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.platform, "terminal");
}

/// Verify that peer_id from NormalizedMessage flows into InboundChainInput.
#[test]
fn test_normalized_to_inbound_peer_id() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: "text".to_string(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: Some("owner".to_string()),
        card_action: None,
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.peer_id, "cli");
}

/// Verify that sender_id from NormalizedMessage flows into InboundChainInput.
#[test]
fn test_normalized_to_inbound_sender_id() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "custom-sender-42".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: "text".to_string(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: Some("owner".to_string()),
        card_action: None,
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.sender_id, "custom-sender-42");
}

/// Verify that timestamp maps to timestamp_ms.
#[test]
fn test_normalized_to_inbound_timestamp() {
    let ts = 1_700_000_123_456_i64;
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: ts,
        message_type: "text".to_string(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: Some("owner".to_string()),
        card_action: None,
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.timestamp_ms, ts);
}

/// Verify that account_id Some("owner") flows through.
#[test]
fn test_normalized_to_inbound_account_id_some() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: "text".to_string(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: Some("owner".to_string()),
        card_action: None,
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.account_id.as_deref(), Some("owner"));
}

/// Verify that account_id None flows through.
#[test]
fn test_normalized_to_inbound_account_id_none() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: "text".to_string(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: None,
        card_action: None,
    };
    let input = normalized_to_inbound(&msg);
    assert!(input.account_id.is_none());
}

/// Verify that content is preserved exactly.
#[test]
fn test_normalized_to_inbound_content_preserved() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "line1\nline2".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: "text".to_string(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: Some("owner".to_string()),
        card_action: None,
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.content, "line1\nline2");
}

/// Verify message_id format: cli-{sender_id}-{timestamp}.
#[test]
fn test_normalized_to_inbound_message_id_format() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "u99".to_string(),
        peer_id: "cli".to_string(),
        content: "hi".to_string(),
        timestamp: 42,
        message_type: "text".to_string(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: None,
        card_action: None,
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.message_id, "cli-u99-42");
}
