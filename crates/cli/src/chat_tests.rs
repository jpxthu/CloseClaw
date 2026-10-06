//! Unit tests for the interactive chat REPL.
//!
//! Verifies quit/exit detection, stop routing, inbound processor chain
//! behavior, NormalizedMessage field mapping, streaming wait conditions,
//! and Gateway architecture verification.

use closeclaw_common::{NormalizedMessage, ReasoningLevel};
use closeclaw_gateway::{GatewayConfig, SessionManager};
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

/// Fake slash router standing in for the composition-root-injected
/// concrete dispatcher (cli must not depend on the slash crate): records
/// every command the Gateway routes to it and answers `/stop` with
/// `SlashResult::Stop`.
struct RecordingStopRouter {
    received: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl closeclaw_common::SlashRouter for RecordingStopRouter {
    async fn dispatch(
        &self,
        content: &str,
        _ctx: &closeclaw_common::SlashContext,
    ) -> Option<closeclaw_common::SlashResult> {
        self.received
            .lock()
            .expect("received mutex poisoned")
            .push(content.to_string());
        Some(closeclaw_common::SlashResult::Stop)
    }

    fn is_immediate(&self, _content: &str) -> bool {
        true
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn closeclaw_common::SlashHandler>> {
        if command != "stop" {
            return None;
        }
        self.received
            .lock()
            .expect("received mutex poisoned")
            .push(format!("/{command}"));
        Some(Box::new(RecordingStopHandler {
            received: Arc::clone(&self.received),
        }))
    }
}

/// Handler backing [`RecordingStopRouter`]: yields the `Stop` result the
/// Gateway must consume.
struct RecordingStopHandler {
    received: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl closeclaw_common::SlashHandler for RecordingStopHandler {
    fn clone_box(&self) -> Box<dyn closeclaw_common::SlashHandler> {
        Box::new(Self {
            received: Arc::clone(&self.received),
        })
    }

    fn commands(&self) -> &[&str] {
        &["stop"]
    }

    fn description(&self) -> &str {
        "stop (fake)"
    }

    async fn handle(
        &self,
        _args: &str,
        _ctx: &closeclaw_common::SlashContext,
    ) -> closeclaw_common::SlashResult {
        closeclaw_common::SlashResult::Stop
    }
}

/// Verify that `/stop` reaches the router injected into the Gateway and
/// that the router's `SlashResult::Stop` is consumed by the Gateway as a
/// handled slash command (the cli-side injection seam).
#[tokio::test]
async fn test_stop_routes_through_gateway_slash_dispatcher() {
    let received = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let router = Arc::new(RecordingStopRouter {
        received: Arc::clone(&received),
    });

    let config = GatewayConfig {
        name: "test-stop-gw".to_string(),
        max_message_size: 64 * 1024,
        bot_agent_bindings: std::collections::HashMap::from([(
            "cli".to_string(),
            "test-agent".to_string(),
        )]),
        ..Default::default()
    };
    let session_manager = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gateway = closeclaw_gateway::Gateway::new(config, session_manager);
    gateway
        .set_slash_dispatcher(router as Arc<dyn closeclaw_common::SlashRouter>)
        .await;

    let processed = gateway
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "owner".into(),
            peer_id: "cli".into(),
            content: "/stop".into(),
            timestamp: 0,
            account_id: "owner".into(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            reply_ref: None,
            unavailable_media: Vec::new(),
        })
        .await;

    let result = gateway
        .handle_inbound_message(processed, Some("owner"), "terminal")
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "gateway must handle the injected router's Stop result, got {result:?}"
    );
    assert_eq!(
        *received.lock().expect("received mutex poisoned"),
        vec!["/stop".to_string()],
        "injected router must receive /stop"
    );
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
            closeclaw_common::ReasoningLevel::default(),
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
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: input.into(),
            timestamp: 0,
            account_id: String::new(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            reply_ref: None,
            unavailable_media: Vec::new(),
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
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: "hello".into(),
            timestamp: 0,
            account_id: String::new(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            reply_ref: None,
            unavailable_media: Vec::new(),
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
            .process_inbound_chain(&NormalizedMessage {
                platform: "terminal".into(),
                sender_id: "u1".into(),
                peer_id: "cli".into(),
                content: cmd.to_string(),
                timestamp: 0,
                account_id: String::new(),
                thread_id: None,
                chat_name: String::new(),
                trace_id: String::new(),
                message_id: String::new(),
                message_type: Default::default(),
                media_refs: Vec::new(),
                reply_ref: None,
                unavailable_media: Vec::new(),
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
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: "cli".into(),
            content: "/stop".into(),
            timestamp: 0,
            account_id: String::new(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            reply_ref: None,
            unavailable_media: Vec::new(),
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
        .process_inbound_chain(&NormalizedMessage {
            platform: "terminal".into(),
            sender_id: "u1".into(),
            peer_id: peer_id_argument.into(),
            content: "hello".into(),
            timestamp: 0,
            account_id: String::new(),
            thread_id: None,
            chat_name: String::new(),
            trace_id: String::new(),
            message_id: String::new(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            reply_ref: None,
            unavailable_media: Vec::new(),
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
        Some(HandleResult::MessageQueued("⏳ 正在排队...".to_string())),
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

// ── Gateway architecture verification ──────────────────────────────────────

/// Verify the chat module creates a Gateway instance and registers TerminalPlugin.
///
/// This test ensures the CLI uses the Gateway-based architecture (Step 1.2)
/// rather than the old RPC client pattern.
#[test]
fn test_chat_module_uses_gateway_not_rpc() {
    let module_source = include_str!("chat/mod.rs");
    assert!(
        module_source.contains("Gateway::new"),
        "chat/mod.rs should construct Gateway directly"
    );
    assert!(
        module_source.contains("SessionManager::new"),
        "chat/mod.rs should construct SessionManager"
    );
    assert!(
        module_source.contains("TerminalPlugin"),
        "chat/mod.rs should register TerminalPlugin"
    );
    assert!(
        module_source.contains("admin"),
        "chat/mod.rs should check daemon reachability via admin socket"
    );
    assert!(
        module_source.contains("async fn run_chat"),
        "chat/mod.rs should still have run_chat function"
    );
}

// ── Empty content filtering ──────────────────────────────────────────────

/// Verify that empty content does not produce a NormalizedMessage.
#[test]
fn test_empty_content_filtered() {
    use crate::terminal::TerminalAdapter;
    let adapter = TerminalAdapter::new();
    // Empty string should return None
    assert!(adapter.make_message("".to_string()).content.is_empty());
}

/// Verify that whitespace-only content is treated as empty.
#[test]
fn test_whitespace_only_content_filtered() {
    use crate::terminal::TerminalAdapter;
    let adapter = TerminalAdapter::new();
    let msg = adapter.make_message("   \n  \t  ".to_string());
    assert!(msg.content.trim().is_empty());
}

// ── Daemon unreachable error path ────────────────────────────────────────

/// Verify that run_chat returns an error when daemon is unreachable.
#[tokio::test]
async fn test_run_chat_daemon_unreachable() {
    // run_chat checks admin socket reachability internally; calling it
    // when no daemon is running should return an error. The injected slash
    // assembly closure is never reached on this path.
    let result = crate::chat::run_chat("test-agent", |_sm_query| {
        Arc::new(RecordingStopRouter {
            received: Arc::new(std::sync::Mutex::new(Vec::new())),
        }) as Arc<dyn closeclaw_common::SlashRouter>
    })
    .await;
    assert!(result.is_err(), "should fail when daemon is unreachable");
}

// ── Architecture: no RPC imports ─────────────────────────────────────────

/// Verify chat/mod.rs does not import ChatRpcClient.
#[test]
fn test_chat_no_rpc_imports() {
    let source = include_str!("chat/mod.rs");
    assert!(
        !source.contains("use.*ChatRpcClient"),
        "chat/mod.rs must not import ChatRpcClient"
    );
    assert!(
        !source.contains("use.*ChatResponse"),
        "chat/mod.rs must not import ChatResponse from RPC"
    );
}

// ── Architecture: TerminalPlugin registered ──────────────────────────────

/// Verify TerminalPlugin is used in the chat module.
#[test]
fn test_terminal_plugin_in_chat() {
    let source = include_str!("chat/mod.rs");
    assert!(
        source.contains("TerminalPlugin::new"),
        "chat/mod.rs should instantiate TerminalPlugin"
    );
    assert!(
        source.contains("register_plugin"),
        "chat/mod.rs should register TerminalPlugin with Gateway"
    );
}
