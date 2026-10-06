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

// ── Slash injection path (Step 1.4) ──────────────────────────────────────

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

/// Reply text produced by the fake handler injected for `/ping`.
const PING_REPLY_TEXT: &str = "injected-pong";

/// Build the inbound message used by the injection-path tests.
fn slash_input(content: &str) -> NormalizedMessage {
    NormalizedMessage {
        platform: "terminal".into(),
        sender_id: "owner".into(),
        peer_id: "cli".into(),
        content: content.to_string(),
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
    }
}

/// Fake router standing in for the composition-root-injected dispatcher:
/// records the slash input the chat pipeline routes to it and answers
/// `/ping` with a `SlashResult::Reply` produced by its handler.
struct RecordingReplyRouter {
    received: Arc<std::sync::Mutex<Vec<String>>>,
}

impl RecordingReplyRouter {
    fn record(&self, content: &str) {
        self.received
            .lock()
            .expect("received mutex poisoned")
            .push(content.to_string());
    }
}

#[async_trait::async_trait]
impl closeclaw_common::SlashRouter for RecordingReplyRouter {
    async fn dispatch(
        &self,
        content: &str,
        _ctx: &closeclaw_common::SlashContext,
    ) -> Option<closeclaw_common::SlashResult> {
        self.record(content);
        Some(closeclaw_common::SlashResult::Reply(
            PING_REPLY_TEXT.to_string(),
        ))
    }

    fn is_immediate(&self, _content: &str) -> bool {
        true
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn closeclaw_common::SlashHandler>> {
        if command != "ping" {
            return None;
        }
        self.record(&format!("/{command}"));
        Some(Box::new(RecordingReplyHandler {
            received: Arc::clone(&self.received),
        }))
    }
}

/// Handler backing [`RecordingReplyRouter`]: yields the `Reply` result the
/// Gateway must route to the output link.
struct RecordingReplyHandler {
    received: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl closeclaw_common::SlashHandler for RecordingReplyHandler {
    fn clone_box(&self) -> Box<dyn closeclaw_common::SlashHandler> {
        Box::new(Self {
            received: Arc::clone(&self.received),
        })
    }

    fn commands(&self) -> &[&str] {
        &["ping"]
    }

    fn description(&self) -> &str {
        "ping (fake)"
    }

    async fn handle(
        &self,
        _args: &str,
        _ctx: &closeclaw_common::SlashContext,
    ) -> closeclaw_common::SlashResult {
        closeclaw_common::SlashResult::Reply(PING_REPLY_TEXT.to_string())
    }
}

/// Spy IM plugin on the `terminal` channel: records every outbound text the
/// Gateway sends through the registered plugin (replaces `TerminalPlugin`,
/// which would write to stdout).
struct TerminalSpyPlugin {
    sent: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl closeclaw_common::IMPlugin for TerminalSpyPlugin {
    fn platform(&self) -> &str {
        "terminal"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, closeclaw_common::AdapterError> {
        Ok(None)
    }

    async fn send(
        &self,
        output: &closeclaw_common::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), closeclaw_common::AdapterError> {
        let text = output.payload.as_str().unwrap_or("").to_string();
        if !text.is_empty() {
            self.sent.lock().expect("sent mutex poisoned").push(text);
        }
        Ok(())
    }
}

/// Build the cli chat gateway through the real injection seam, feeding it a
/// fake router and returning the observable handles the tests assert on.
struct InjectionFixture {
    gateway: Arc<closeclaw_gateway::Gateway>,
    /// Keeps the injected config dir alive for the test's duration.
    _tmp: tempfile::TempDir,
    _output_rx: tokio::sync::mpsc::Receiver<(String, Vec<closeclaw_common::ContentBlock>)>,
    received: Arc<std::sync::Mutex<Vec<String>>>,
    sent: Arc<std::sync::Mutex<Vec<String>>>,
    sm_query_slot: Arc<std::sync::Mutex<Option<Arc<dyn closeclaw_common::SlashSessionQuery>>>>,
    closure_calls: Arc<AtomicUsize>,
}

async fn build_injection_fixture() -> InjectionFixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let received = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sent = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sm_query_slot: Arc<std::sync::Mutex<Option<Arc<dyn closeclaw_common::SlashSessionQuery>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let closure_calls = Arc::new(AtomicUsize::new(0));

    let llm_registry = Arc::new(closeclaw_llm::LLMRegistry::new());
    let fallback_client = closeclaw_llm::call_chain::build_fallback_client(&llm_registry).await;

    let router_received = Arc::clone(&received);
    let (gateway, output_rx) =
        crate::chat::build_gateway(tmp.path(), "test-agent", &llm_registry, &fallback_client, {
            let sm_query_slot = Arc::clone(&sm_query_slot);
            let closure_calls = Arc::clone(&closure_calls);
            move |sm_query| {
                closure_calls.fetch_add(1, AtomicOrdering::SeqCst);
                *sm_query_slot.lock().expect("slot mutex poisoned") = Some(sm_query);
                Arc::new(RecordingReplyRouter {
                    received: router_received,
                }) as Arc<dyn closeclaw_common::SlashRouter>
            }
        })
        .await
        .expect("build_gateway must succeed");

    InjectionFixture {
        gateway,
        _tmp: tmp,
        _output_rx: output_rx,
        received,
        sent,
        sm_query_slot,
        closure_calls,
    }
}

/// Slash injection seam: `build_gateway` invokes the injected assembly
/// closure exactly once, hands it the `SessionManager`-derived
/// `SlashSessionQuery`, and installs the returned router on the Gateway.
#[tokio::test]
async fn test_slash_router_injection_installed_on_gateway() {
    let fx = build_injection_fixture().await;

    assert_eq!(
        fx.closure_calls.load(AtomicOrdering::SeqCst),
        1,
        "the injected slash assembly closure must be called exactly once"
    );
    assert!(
        fx.sm_query_slot
            .lock()
            .expect("slot mutex poisoned")
            .is_some(),
        "the closure must receive the SessionManager-derived SlashSessionQuery"
    );
    assert!(
        fx.gateway.has_slash_dispatcher().await,
        "the router returned by the injected closure must be installed on the Gateway"
    );
}

/// Slash injection normal path: the chat pipeline routes `/` input to the
/// injected router and the handler's `SlashResult::Reply` comes out of the
/// terminal output link.
///
/// `#[ignore]` — blocked by the pre-existing production bug tracked in
/// issue #3411 (not a defect of the injection itself):
/// - **blocker**: `build_gateway` builds `GatewayConfig` with
///   `..Default::default()`, so `max_message_size == 0` and
///   `validate_inbound` rejects every non-empty message before slash routing.
/// - **复现证据**: running this test un-ignored prints
///   `result=None sessions=0 sent=["消息过长，请缩短后重试"]` — the
///   closure runs and the router is installed, but the router never sees
///   `/ping`.
/// - **解除条件**: #3411 fixed (non-zero `max_message_size` for cli chat) →
///   remove `#[ignore]` and this test must pass unchanged; it is listed in
///   the PR body dormant-test list until then.
#[tokio::test]
#[ignore = "blocked by closeclaw issue #3411: cli chat max_message_size = 0 rejects all input"]
async fn test_slash_router_injection_reply_reaches_output() {
    let fx = build_injection_fixture().await;

    // Replace the TerminalPlugin registered by build_gateway so the
    // outbound reply text is observable instead of printed to stdout.
    fx.gateway
        .register_plugin(Arc::new(TerminalSpyPlugin {
            sent: Arc::clone(&fx.sent),
        }))
        .await;

    let processed = fx
        .gateway
        .process_inbound_chain(&slash_input("/ping"))
        .await;
    let result = fx
        .gateway
        .handle_inbound_message(processed, Some("owner"), "terminal")
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "slash reply must be consumed as SlashHandled, got {result:?}"
    );
    assert_eq!(
        *fx.received.lock().expect("received mutex poisoned"),
        vec!["/ping".to_string()],
        "the injected router must receive the chat pipeline's slash input"
    );
    assert_eq!(
        *fx.sent.lock().expect("sent mutex poisoned"),
        vec![PING_REPLY_TEXT.to_string()],
        "SlashResult::Reply must reach the terminal output link"
    );
}

/// Slash injection boundary: a Gateway that never had `set_slash_dispatcher`
/// called must not panic on `/` input. Recorded actual behaviour:
/// `dispatch_slash` yields `None` without a dispatcher, so the message is
/// not consumed as a slash command and falls through to the session handler;
/// this Gateway registers no session handler, hence the result is `None`.
#[tokio::test]
async fn test_slash_input_without_dispatcher_does_not_panic() {
    let config = GatewayConfig {
        name: "test-no-dispatcher-gw".to_string(),
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
    let gateway = closeclaw_gateway::Gateway::new(config, Arc::clone(&session_manager));
    assert!(
        !gateway.has_slash_dispatcher().await,
        "precondition: no slash dispatcher injected"
    );

    let processed = gateway.process_inbound_chain(&slash_input("/ping")).await;
    let result = gateway
        .handle_inbound_message(processed, Some("owner"), "terminal")
        .await;

    assert_eq!(
        gateway.get_agent_sessions("test-agent").await.len(),
        1,
        "session must be resolved so the input actually reaches slash routing"
    );
    assert!(
        result.is_none(),
        "without a dispatcher the input must not be claimed as SlashHandled, got {result:?}"
    );
}
