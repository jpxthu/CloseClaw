//! Unit tests for the cli chat Gateway's slash injection seam.
//!
//! The composition root assembles the concrete slash handler set and injects
//! it into `build_gateway`; these tests drive that seam with a parameterized
//! fake router, so the cli crate keeps zero dependency on the slash crate.

use closeclaw_common::{
    NormalizedMessage, ReasoningLevel, SlashContext, SlashHandler, SlashResult, SlashRouter,
    SlashSessionQuery,
};
use closeclaw_gateway::{GatewayConfig, HandleResult, SessionManager};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

/// Reply text produced by the fake handler injected for `/ping`.
const PING_REPLY_TEXT: &str = "injected-pong";

/// Factory for the [`SlashResult`] a fake handler returns. `SlashResult`
/// does not implement `Clone`, so the fakes hold a factory, not a value.
type ResultFactory = Arc<dyn Fn() -> SlashResult + Send + Sync>;

/// Parameterized fake standing in for the composition-root-injected
/// dispatcher (the cli crate must not depend on the slash crate): claims
/// exactly `command`, answers it with `make_result()`, and records every
/// command the Gateway resolves through [`SlashRouter::get_handler`].
///
/// The Gateway production path resolves the handler via `get_handler` and
/// produces the [`SlashResult`] inside `handler.handle()`; the `dispatch`
/// method of `SlashRouter` is not called on that path, so it returns `None`
/// here (same shape as `ResultRouter` in `crates/gateway/src/tests_slash_permission.rs`).
struct RecordingRouter {
    command: &'static str,
    make_result: ResultFactory,
    received: Arc<Mutex<Vec<String>>>,
}

/// Build the parameterized fake router answering `command` with
/// `make_result()`. `received` collects every `/command` the Gateway
/// resolves through `get_handler`, i.e. what the injected router observed.
pub(crate) fn recording_router(
    command: &'static str,
    make_result: ResultFactory,
    received: Arc<Mutex<Vec<String>>>,
) -> Arc<dyn SlashRouter> {
    Arc::new(RecordingRouter {
        command,
        make_result,
        received,
    })
}

#[async_trait::async_trait]
impl SlashRouter for RecordingRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }

    fn is_immediate(&self, _content: &str) -> bool {
        true
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command != self.command {
            return None;
        }
        self.received
            .lock()
            .expect("received mutex poisoned")
            .push(format!("/{command}"));
        Some(Box::new(RecordingHandler {
            command: self.command,
            make_result: Arc::clone(&self.make_result),
        }))
    }
}

/// Handler backing [`RecordingRouter`]: produces the configured
/// [`SlashResult`] that the Gateway routes onward.
struct RecordingHandler {
    command: &'static str,
    make_result: ResultFactory,
}

#[async_trait::async_trait]
impl SlashHandler for RecordingHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(Self {
            command: self.command,
            make_result: Arc::clone(&self.make_result),
        })
    }

    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }

    fn description(&self) -> &str {
        "fake handler (test only)"
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        (self.make_result)()
    }
}

/// The default CLI chat processor chain — the same inbound/outbound set the
/// composition root injects in production for a `GatewayConfig` with
/// `raw_log_dir = None`. Assembled from the cli crate's dev-dependency on
/// `closeclaw-processor-chain`, so the tests drive the seam with a real
/// chain instead of the composition root's single implementation.
pub(crate) fn chat_processor_chain() -> Arc<dyn closeclaw_common::processor::ProcessorChain> {
    use closeclaw_processor_chain::content_normalizer::ContentNormalizer;
    use closeclaw_processor_chain::session_router::SessionRouter;
    use closeclaw_processor_chain::verbosity_filter::VerbosityFilter;
    use closeclaw_processor_chain::{DslParser, ProcessorRegistry};

    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(SessionRouter::new()));
    registry.register(Arc::new(ContentNormalizer::new()));
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    Arc::new(registry)
}

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

/// Verify that `/stop` reaches the router injected into the Gateway and
/// that the router's `SlashResult::Stop` is consumed by the Gateway as a
/// handled slash command (the cli-side injection seam).
#[tokio::test]
async fn test_stop_routes_through_gateway_slash_dispatcher() {
    let received = Arc::new(Mutex::new(Vec::<String>::new()));
    let router = recording_router("stop", Arc::new(|| SlashResult::Stop), received.clone());

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
    let gateway = closeclaw_gateway::Gateway::new(config, session_manager, chat_processor_chain());
    gateway.set_slash_dispatcher(router).await;

    let processed = gateway.process_inbound_chain(&slash_input("/stop")).await;
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

/// Spy IM plugin on the `terminal` channel: records every outbound text the
/// Gateway sends through the registered plugin (replaces `TerminalPlugin`,
/// which would write to stdout).
struct TerminalSpyPlugin {
    sent: Arc<Mutex<Vec<String>>>,
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
    received: Arc<Mutex<Vec<String>>>,
    sent: Arc<Mutex<Vec<String>>>,
    sm_query_slot: Arc<Mutex<Option<Arc<dyn SlashSessionQuery>>>>,
    closure_calls: Arc<AtomicUsize>,
    chain_calls: Arc<AtomicUsize>,
}

async fn build_injection_fixture() -> InjectionFixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let received = Arc::new(Mutex::new(Vec::<String>::new()));
    let sent = Arc::new(Mutex::new(Vec::<String>::new()));
    let sm_query_slot: Arc<Mutex<Option<Arc<dyn SlashSessionQuery>>>> = Arc::new(Mutex::new(None));
    let closure_calls = Arc::new(AtomicUsize::new(0));
    let chain_calls = Arc::new(AtomicUsize::new(0));

    let llm_registry = Arc::new(closeclaw_llm::LLMRegistry::new());
    let fallback_client = closeclaw_llm::call_chain::build_fallback_client(&llm_registry).await;

    let router_received = Arc::clone(&received);
    let chain_calls_for_closure = Arc::clone(&chain_calls);
    let (gateway, output_rx) = crate::chat::build_gateway(
        tmp.path(),
        "test-agent",
        &llm_registry,
        &fallback_client,
        {
            let sm_query_slot = Arc::clone(&sm_query_slot);
            let closure_calls = Arc::clone(&closure_calls);
            move |sm_query| {
                closure_calls.fetch_add(1, AtomicOrdering::SeqCst);
                *sm_query_slot.lock().expect("slot mutex poisoned") = Some(sm_query);
                recording_router(
                    "ping",
                    Arc::new(|| SlashResult::Reply(PING_REPLY_TEXT.to_string())),
                    router_received,
                )
            }
        },
        move |_config| {
            chain_calls_for_closure.fetch_add(1, AtomicOrdering::SeqCst);
            chat_processor_chain()
        },
    )
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
        chain_calls,
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

/// Processor-chain injection seam: `build_gateway` invokes the injected
/// assembly closure exactly once and installs the returned chain on the
/// Gateway — the cli crate itself never builds concrete processors.
#[tokio::test]
async fn test_processor_chain_injection_installed_on_gateway() {
    let fx = build_injection_fixture().await;

    assert_eq!(
        fx.chain_calls.load(AtomicOrdering::SeqCst),
        1,
        "the injected processor-chain assembly closure must be called exactly once"
    );
    let (inbound, outbound) = fx.gateway.processor_registry_len();
    assert!(
        inbound > 0 && outbound > 0,
        "the injected chain must be installed, got ({inbound}, {outbound})"
    );
}

/// Slash injection normal path: the chat pipeline routes `/` input to the
/// injected router and the handler's `SlashResult::Reply` comes out of the
/// terminal output link.
///
/// `#[ignore]` — blocked by the pre-existing production bug tracked in
/// issue #3411 (not a defect of the injection itself):
/// - **blocker**: `build_gateway` builds `GatewayConfig` with
///   `..Default::default()` (`crates/cli/src/chat/mod.rs`), so
///   `max_message_size == 0` and `validate_inbound`
///   (`crates/gateway/src/media_routing.rs`) rejects every non-empty message
///   with `InboundValidation::RejectSilently` before slash routing.
/// - **复现证据**: run the following command
///   `cargo nextest run -p closeclaw-cli slash_router_injection_reply --run-ignored only`
///   → panics at `crates/cli/src/chat_slash_injection_tests.rs:325:5` with
///   `SlashHandled expected, got None; received=[] sent=["消息过长，请缩短后重试"]`
///   — the closure runs and the router is installed, but the router never
///   sees `/ping`.
/// - **解除条件**: #3411 fixed (non-zero `max_message_size` for cli chat) →
///   remove `#[ignore]` and this test must pass unchanged; until then it
///   will be listed in the PR body dormant-test list.
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

    let received = fx.received.lock().expect("received mutex poisoned");
    let sent = fx.sent.lock().expect("sent mutex poisoned");

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "SlashHandled expected, got {result:?}; received={received:?} sent={sent:?}",
    );
    assert_eq!(
        *received,
        vec!["/ping".to_string()],
        "the injected router must receive the chat pipeline's slash input"
    );
    assert_eq!(
        *sent,
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
    let gateway = closeclaw_gateway::Gateway::new(
        config,
        Arc::clone(&session_manager),
        chat_processor_chain(),
    );
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
