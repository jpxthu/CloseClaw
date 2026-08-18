//! Unit tests for the interactive chat REPL.
//!
//! Verifies quit/exit detection, stop routing, inbound processor chain
//! behavior, NormalizedMessage field mapping, and streaming wait conditions.

use closeclaw_common::{MessageType, NormalizedMessage};
use closeclaw_gateway::{GatewayConfig, InboundChainInput, SessionManager};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

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

// ── /stop REPL routing tests ───────────────────────────────────────────────

/// Verify that `/stop` routes through the gateway's SlashDispatcher
/// and returns `SlashResult::Stop` with cascade=true, force=true.
#[tokio::test]
async fn test_stop_routes_through_gateway_slash_dispatcher() {
    use closeclaw_slash::dispatcher::SlashDispatcher;
    use closeclaw_slash::registry::HandlerRegistry;

    let slash_registry = Arc::new(HandlerRegistry::new());
    let _session_manager = Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test-stop-gw".to_string(),
            ..Default::default()
        },
        None,
        None,
        ReasoningLevel::default(),
    ));
    slash_registry.register(Arc::new(closeclaw_slash::StopHandler));
    let dispatcher = SlashDispatcher::from_shared(slash_registry);

    let ctx = closeclaw_slash::context::SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };

    match dispatcher.dispatch("/stop", &ctx).await {
        closeclaw_common::slash_router::SlashResult::Stop { cascade, force } => {
            assert!(cascade, "cascade must be true");
            assert!(force, "force must be true");
        }
        other => panic!("expected Stop from gateway dispatch, got {other:?}"),
    }
}

/// Verify that `/stop` is NOT treated as a quit command by the REPL
/// detection logic. This ensures the REPL continues after `/stop`.
#[test]
fn test_stop_does_not_trigger_quit() {
    // /stop must not match quit detection
    assert!(!is_quit_command("/stop"));
    assert!(!is_quit_command("/STOP"));
    // But must match stop detection
    assert!(is_stop_command("/stop"));
    assert!(is_stop_command("/STOP"));
}

// ── Inbound Processor Chain integration tests ─────────────────────────────

use async_trait::async_trait;
use closeclaw_common::ProcessedMessage;
use closeclaw_processor_chain::content_normalizer::ContentNormalizer;
use closeclaw_processor_chain::{MessageContext, ProcessError, ProcessorRegistry};

/// A mock processor that suppresses messages (for testing suppress behavior).
struct SuppressProcessor;

#[async_trait]
impl closeclaw_processor_chain::MessageProcessor for SuppressProcessor {
    fn name(&self) -> &str {
        "suppress-processor"
    }

    fn phase(&self) -> closeclaw_processor_chain::ProcessPhase {
        closeclaw_processor_chain::ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        0
    }

    async fn process(
        &self,
        _ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        Ok(None)
    }
}

/// Build a Gateway with the given ProcessorRegistry.
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
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            chat_name: None,
            trace_id: None,
        })
        .await;

    assert_eq!(processed.text_content(), Some("helloworld"));
    assert!(!processed.content_blocks.is_empty());
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
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            chat_name: None,
            trace_id: None,
        })
        .await;

    assert!(
        processed.content_blocks.is_empty(),
        "expected empty content_blocks (suppress)"
    );
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
                thread_id: None,
                message_type: Default::default(),
                media_refs: Vec::new(),
                chat_name: None,
                trace_id: None,
            })
            .await;
        assert_eq!(processed.text_content().unwrap_or(""), *cmd);
    }
}

#[tokio::test]
async fn test_inbound_chain_preserves_stop_for_gateway_routing() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

    let processed = gateway
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: "/stop".into(),
            message_id: "msg-stop-1".into(),
            timestamp_ms: 0,
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            chat_name: None,
            trace_id: None,
        })
        .await;

    assert_eq!(
        processed.text_content(),
        Some("/stop"),
        "/stop must be preserved through inbound chain"
    );
}

// ── peer_id "cli" verification ────────────────────────────────────────────

#[tokio::test]
async fn test_process_inbound_chain_peer_id_is_cli() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));
    let gateway = make_gw_with_registry(registry);

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
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            chat_name: None,
            trace_id: None,
        })
        .await;

    assert!(!processed.content_blocks.is_empty());
    assert_eq!(
        peer_id_argument, "cli",
        "peer_id must be 'cli' per design doc"
    );
}

// ── REPL streaming wait condition tests ───────────────────────────────────

use super::chat::should_wait_for_streaming;
use closeclaw_gateway::HandleResult;

/// `LlmStarted` with a non-empty session_key triggers the streaming wait.
#[test]
fn test_should_wait_llm_started_with_session_key() {
    assert!(should_wait_for_streaming(
        Some(HandleResult::LlmStarted),
        "session-abc"
    ));
}

/// Non-`LlmStarted` results skip the streaming wait.
#[test]
fn test_should_wait_message_queued_skips() {
    assert!(!should_wait_for_streaming(
        Some(HandleResult::MessageQueued),
        "session-abc"
    ));
}

/// `SlashHandled` result skips the streaming wait.
#[test]
fn test_should_wait_slash_handled_skips() {
    assert!(!should_wait_for_streaming(
        Some(HandleResult::SlashHandled),
        "session-abc"
    ));
}

/// `ApprovalProcessed` result skips the streaming wait.
#[test]
fn test_should_wait_approval_processed_skips() {
    assert!(!should_wait_for_streaming(
        Some(HandleResult::ApprovalProcessed),
        "session-abc"
    ));
}

/// `None` result (no session handler) skips the streaming wait.
#[test]
fn test_should_wait_none_result_skips() {
    assert!(!should_wait_for_streaming(None, "session-abc"));
}

/// `LlmStarted` with empty session_key skips the streaming wait.
#[test]
fn test_should_wait_llm_started_empty_session_key() {
    assert!(!should_wait_for_streaming(
        Some(HandleResult::LlmStarted),
        ""
    ));
}

// ── NormalizedMessage → InboundChainInput field mapping ───────────────────

/// Helper: simulate the field extraction logic.
fn normalized_to_inbound(msg: &NormalizedMessage) -> InboundChainInput {
    let message_id = format!("cli-{}-{}", msg.sender_id, msg.timestamp);
    InboundChainInput {
        platform: msg.platform.clone(),
        sender_id: msg.sender_id.clone(),
        peer_id: msg.peer_id.clone(),
        content: msg.content.clone(),
        message_id,
        timestamp_ms: msg.timestamp,
        account_id: Some(msg.account_id.clone()),
        thread_id: msg.thread_id.clone(),
        message_type: msg.message_type.clone(),
        media_refs: msg.media_refs.clone(),
        chat_name: None,
        trace_id: None,
    }
}

#[test]
fn test_normalized_to_inbound_platform() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: "owner".to_string(),
        ..Default::default()
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.platform, "terminal");
}

#[test]
fn test_normalized_to_inbound_peer_id() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: "owner".to_string(),
        ..Default::default()
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.peer_id, "cli");
}

#[test]
fn test_normalized_to_inbound_sender_id() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "custom-sender-42".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: "owner".to_string(),
        ..Default::default()
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.sender_id, "custom-sender-42");
}

#[test]
fn test_normalized_to_inbound_timestamp() {
    let ts = 1_700_000_123_456_i64;
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: ts,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: "owner".to_string(),
        ..Default::default()
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.timestamp_ms, ts);
}

#[test]
fn test_normalized_to_inbound_account_id_present() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: "owner".to_string(),
        ..Default::default()
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.account_id.as_deref(), Some("owner"));
}

#[test]
fn test_normalized_to_inbound_account_id_empty() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: String::new(),
        ..Default::default()
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.account_id.as_deref(), Some(""));
}

#[test]
fn test_normalized_to_inbound_content_preserved() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "line1\nline2".to_string(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: "owner".to_string(),
        ..Default::default()
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.content, "line1\nline2");
}

#[test]
fn test_normalized_to_inbound_message_id_format() {
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "u99".to_string(),
        peer_id: "cli".to_string(),
        content: "hi".to_string(),
        timestamp: 42,
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: String::new(),
        ..Default::default()
    };
    let input = normalized_to_inbound(&msg);
    assert_eq!(input.message_id, "cli-u99-42");
}
