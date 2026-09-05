//! Step 1.9 — Chain behavior dimension tests for the gateway pre-call
//! reasoning level resolution.
//!
//! Verifies the full resolution chain end-to-end with a real `Gateway`
//! wired to a `SessionMessageHandler` carrying `ProviderModelKnowledge`
//! (mirrors the daemon composition-root wiring from Step 1.3):
//!
//! - 正常路径: KB hit (glm-5.1 / deepseek-v4-flash / MiniMax-M3) request
//!   Max → session effective level High, request carries High, plugin maps
//!   High to native parameters (thinking enabled / effort "high")
//! - 状态转换: `/reasoning` change → next call resolves with the new level;
//!   `set_reasoning_level` resets the effective level
//! - 错误路径/边界: unknown non-anthropic model passthrough, claude
//!   heuristic fallback (Off→Low, non-High→High, High stays), Off on a
//!   `reasoning_always` model → Low, `ReasoningLevels::None` passthrough
//! - 长链场景: non-streaming and streaming entries both resolve BEFORE the
//!   invoke — verified by driving real `invoke_llm` / `invoke_llm_streaming`
//!   against a capturing fake caller and asserting the captured request
//!   carries the effective level, plus the no-resolve counterfactual
//!
//! See `docs/design/session/llm-session-enhancements.md` §实际生效档位.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_llm::knowledge::{ModelRecommendParams, ProviderModelKnowledge, ReasoningLevels};
use closeclaw_llm::model_info::InputType;
use closeclaw_llm::plugin::{ModelPlugin, PluginPipeline};
use closeclaw_llm::protocol::OpenAiProtocol;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::types::{InternalRequest, ProtocolId};
use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};
use closeclaw_llm::{
    DeepSeekPlugin, GlmPlugin, InterpreterRegistry, MiniMaxM3Plugin, StubProvider,
    UnifiedChatClient,
};
use serde_json::json;

use crate::session_handler::ActiveSearcherLlmCaller;
use crate::session_handler_reasoning::resolve_before_llm_call;
use crate::{Gateway, GatewayConfig, SessionManager, SessionMessageHandler};
use closeclaw_common::llm_types::{InternalMessage, SystemBlock, ToolDefinition};
use closeclaw_common::processor::{
    ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedResponse, UnifiedUsage,
};
use closeclaw_common::{LLMError, LlmCaller, ReasoningLevel};
use closeclaw_session::llm_session::ConversationSession;

// ── shared fixtures ──────────────────────────────────────────────────────

const SESSION_ID: &str = "test-session";

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        ..Default::default()
    }
}

fn workdir() -> std::path::PathBuf {
    std::env::temp_dir().join("closeclaw-reasoning-chain-tests")
}

/// Capture-only LLM caller: records the last request per entry point and
/// returns a canned response / minimal valid stream. No network involved.
#[derive(Default)]
struct CapturingCaller {
    last_non_streaming: std::sync::Mutex<Option<InternalRequest>>,
    last_streaming: std::sync::Mutex<Option<InternalRequest>>,
}

impl CapturingCaller {
    fn last_non_streaming(&self) -> Option<InternalRequest> {
        self.last_non_streaming.lock().unwrap().clone()
    }

    #[allow(dead_code)] // kept for streaming-entry request assertions
    fn last_streaming(&self) -> Option<InternalRequest> {
        self.last_streaming.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmCaller for CapturingCaller {
    async fn call(&self, request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        *self.last_non_streaming.lock().unwrap() = Some(request);
        Ok(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("ok".into())],
            usage: UnifiedUsage::default(),
            finish_reason: Some("stop".into()),
            retry_attempts: 0,
        })
    }

    async fn call_streaming(
        &self,
        request: InternalRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
        LLMError,
    > {
        *self.last_streaming.lock().unwrap() = Some(request);
        let events: Vec<Result<StreamEvent, LLMError>> = vec![
            Ok(StreamEvent::BlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
            Ok(StreamEvent::BlockDelta {
                index: 0,
                delta: ContentDelta::Text {
                    text: "ok".to_string(),
                },
            }),
            Ok(StreamEvent::BlockEnd {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
            Ok(StreamEvent::MessageEnd {
                usage: None,
                finish_reason: Some("stop".to_string()),
            }),
        ];
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

/// Minimal valid [`InternalRequest`] for plugin mapping assertions.
fn make_plugin_request(model: &str, level: ReasoningLevel) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            ..Default::default()
        }],
        temperature: 0.7,
        max_tokens: Some(256),
        stream: false,
        extra_body: Default::default(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None::<Vec<SystemBlock>>,
        tools: None::<Vec<ToolDefinition>>,
        session_id: None,
        reasoning_level: level,
        turn_count: None,
    }
}

/// Build a `SessionManager` with one conversation session (`model`,
/// `requested` level, capturing caller injected) and a real `Gateway`
/// whose handler carries `kb` as model knowledge — the same wiring the
/// daemon composition root performs in production.
/// Fixture bundle. The `Arc<Gateway>` must be kept alive for the test's
/// duration: `SessionManager` only holds a `Weak` back-reference (same as
/// production, where the daemon owns the strong reference).
struct WiredFixture {
    sm: Arc<SessionManager>,
    caller: Arc<CapturingCaller>,
    /// Strong reference keeping the Weak gateway_ref in `sm` resolvable.
    _gateway: Arc<Gateway>,
}

async fn make_wired_sm(
    model: &str,
    requested: ReasoningLevel,
    kb: ProviderModelKnowledge,
) -> WiredFixture {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    let caller = Arc::new(CapturingCaller::default());
    let mut cs = ConversationSession::new(SESSION_ID.to_string(), model.to_string(), workdir());
    cs.set_reasoning_level(requested);
    cs.set_llm_caller(caller.clone());
    sm.conversation_sessions.write().await.insert(
        SESSION_ID.to_string(),
        Arc::new(tokio::sync::RwLock::new(cs)),
    );

    // Real Gateway + handler with knowledge (daemon composition-root wiring).
    let gw = Gateway::new(test_config(), sm.clone());
    let provider: Arc<dyn closeclaw_llm::provider::Provider> = Arc::new(StubProvider::new());
    let client = Arc::new(UnifiedChatClient::new(
        provider,
        Arc::new(OpenAiProtocol::new()),
        InterpreterRegistry::default(),
        PluginPipeline::new(),
        Arc::new(closeclaw_llm::cache_adapter::NoopCacheAdapter),
    ));
    let fallback_client = Arc::new(UnifiedFallbackClient::new(
        vec![ChainEntry {
            provider_id: "stub".into(),
            model_id: "stub".into(),
            client,
        }],
        Arc::new(CooldownManager::new()),
    ));
    let active_searcher = Arc::new(ActiveSearcherLlmCaller {
        caller: Arc::new(crate::llm_caller_impl::FallbackLlmCaller(
            fallback_client.clone(),
        )) as Arc<dyn LlmCaller>,
        model: String::new(),
    });
    let handler = Arc::new(
        SessionMessageHandler::new_no_output(
            sm.clone(),
            fallback_client,
            active_searcher,
            closeclaw_common::CompactConfig::default(),
        )
        .with_model_knowledge(kb),
    );
    let gateway = Arc::new(gw);
    gateway.set_session_handler(handler);
    sm.set_gateway_ref(gateway.clone()).await;
    WiredFixture {
        sm,
        caller,
        _gateway: gateway,
    }
}

/// Custom KB params for edge-case models.
fn custom_model_params(levels: ReasoningLevels, reasoning_always: bool) -> ModelRecommendParams {
    ModelRecommendParams {
        context_window: 128_000,
        max_tokens: 8_192,
        default_temperature: 0.7,
        reasoning: !matches!(levels, ReasoningLevels::None),
        reasoning_levels: levels,
        reasoning_always,
        input_types: vec![InputType::Text],
        recommended_protocol: ProtocolId::from("openai"),
    }
}

/// Assert the session's effective level after resolution.
async fn assert_effective(sm: &Arc<SessionManager>, expected: ReasoningLevel) {
    let cs = sm.get_conversation_session(SESSION_ID).await.unwrap();
    assert_eq!(cs.read().await.effective_reasoning_level(), expected);
}

// ── 正常路径: KB hit → effective High → request High → plugin mapping ────

/// glm-5.1 (Toggle on=true) requests Max → effective High;
/// GLM plugin maps High → thinking.type = "enabled".
#[tokio::test]
async fn test_normal_glm_max_resolves_high_and_plugin_maps_enabled() {
    let fx = make_wired_sm(
        "glm-5.1",
        ReasoningLevel::Max,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;

    assert_effective(sm, ReasoningLevel::High).await;

    let mut req = make_plugin_request("glm-5.1", ReasoningLevel::High);
    GlmPlugin.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"}))
    );
}

/// deepseek-v4-flash (Levels off=true) requests Max → effective High;
/// DeepSeek plugin maps High → reasoning_effort = "high".
#[tokio::test]
async fn test_normal_deepseek_max_resolves_high_and_plugin_maps_effort() {
    let fx = make_wired_sm(
        "deepseek-v4-flash",
        ReasoningLevel::Max,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;

    assert_effective(sm, ReasoningLevel::High).await;

    let mut req = make_plugin_request("deepseek-v4-flash", ReasoningLevel::High);
    DeepSeekPlugin.before_request(&mut req);
    assert_eq!(req.extra_body.get("reasoning_effort"), Some(&json!("high")));
}

/// MiniMax-M3 (Toggle on=true) requests Max → effective High;
/// MiniMaxM3Plugin maps High → thinking.type = "enabled".
#[tokio::test]
async fn test_normal_minimax_max_resolves_high_and_plugin_maps_enabled() {
    let fx = make_wired_sm(
        "MiniMax-M3",
        ReasoningLevel::Max,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;

    assert_effective(sm, ReasoningLevel::High).await;

    let mut req = make_plugin_request("MiniMax-M3", ReasoningLevel::High);
    MiniMaxM3Plugin.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"}))
    );
}

// ── 状态转换: /reasoning change → next call uses new resolution ──────────

/// After resolving Max → High, `/reasoning Medium` resets the effective
/// level and the next pre-call resolve passes Medium through (Toggle on=true).
#[tokio::test]
async fn test_transition_reasoning_change_next_call_uses_new_level() {
    let fx = make_wired_sm(
        "glm-5.1",
        ReasoningLevel::Max,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;
    assert_effective(sm, ReasoningLevel::High).await;

    // /reasoning Medium → set_reasoning_level resets effective to None.
    let cs = sm.get_conversation_session(SESSION_ID).await.unwrap();
    cs.write().await.set_reasoning_level(ReasoningLevel::Medium);
    assert_eq!(
        cs.read().await.effective_reasoning_level(),
        ReasoningLevel::Medium,
        "effective falls back to requested after reset"
    );

    resolve_before_llm_call(sm, SESSION_ID).await;
    assert_effective(sm, ReasoningLevel::Medium).await;
}

/// set_reasoning_level reset semantics: effective falls back to the new
/// requested level immediately, and re-resolution keeps the new level.
#[tokio::test]
async fn test_transition_set_reasoning_level_resets_effective() {
    let fx = make_wired_sm(
        "glm-5.1",
        ReasoningLevel::Max,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;
    assert_effective(sm, ReasoningLevel::High).await;

    let cs = sm.get_conversation_session(SESSION_ID).await.unwrap();
    cs.write().await.set_reasoning_level(ReasoningLevel::Low);
    assert_eq!(
        cs.read().await.effective_reasoning_level(),
        ReasoningLevel::Low
    );

    resolve_before_llm_call(sm, SESSION_ID).await;
    assert_effective(sm, ReasoningLevel::Low).await;
}

// ── 错误路径/边界 ────────────────────────────────────────────────────────

/// Unknown model outside the KB and not anthropic → requested passthrough.
#[tokio::test]
async fn test_boundary_non_kb_non_anthropic_passthrough() {
    let fx = make_wired_sm("gpt-4o", ReasoningLevel::Max, ProviderModelKnowledge::new()).await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;

    assert_effective(sm, ReasoningLevel::Max).await;
}

/// Claude model outside the KB → heuristic fallback Off → Low.
#[tokio::test]
async fn test_boundary_claude_off_heuristic_to_low() {
    let fx = make_wired_sm(
        "claude-3-5-sonnet",
        ReasoningLevel::Off,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;

    assert_effective(sm, ReasoningLevel::Low).await;
}

/// Claude model outside the KB → heuristic fallback Medium → High.
#[tokio::test]
async fn test_boundary_claude_medium_heuristic_to_high() {
    let fx = make_wired_sm(
        "claude-3-opus",
        ReasoningLevel::Medium,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;

    assert_effective(sm, ReasoningLevel::High).await;
}

/// Claude model outside the KB → heuristic keeps already-supported High.
#[tokio::test]
async fn test_boundary_claude_high_heuristic_stays_high() {
    let fx = make_wired_sm(
        "claude-sonnet-4",
        ReasoningLevel::High,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;

    assert_effective(sm, ReasoningLevel::High).await;
}

/// Off on a custom `reasoning_always` model → downgrade to Low.
#[tokio::test]
async fn test_boundary_off_on_reasoning_always_model_downgrades_to_low() {
    let kb = ProviderModelKnowledge::new().with_test_model(
        "test_provider",
        "always-reason-model",
        custom_model_params(ReasoningLevels::Toggle { on: true }, true),
    );
    let fx = make_wired_sm("always-reason-model", ReasoningLevel::Off, kb).await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());

    resolve_before_llm_call(sm, SESSION_ID).await;

    assert_effective(sm, ReasoningLevel::Low).await;
}

/// `ReasoningLevels::None` model → all requested levels pass through.
#[tokio::test]
async fn test_boundary_reasoning_levels_none_passthrough() {
    for requested in [
        ReasoningLevel::Off,
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::Max,
    ] {
        let kb = ProviderModelKnowledge::new().with_test_model(
            "test_provider",
            "no-reason-model",
            custom_model_params(ReasoningLevels::None, false),
        );
        let fx = make_wired_sm("no-reason-model", requested, kb).await;
        let (sm, _caller) = (&fx.sm, fx.caller.clone());
        resolve_before_llm_call(sm, SESSION_ID).await;
        assert_effective(sm, requested).await;
    }
}

/// deepseek-v4-pro (Levels off=false, base=true, reasoner=false) boundaries:
/// Off→Low, Low→Low, Medium→Medium, High→Medium, Max→High.
#[tokio::test]
async fn test_boundary_deepseek_pro_levels_boundaries() {
    let cases = [
        (ReasoningLevel::Off, ReasoningLevel::Low),
        (ReasoningLevel::Low, ReasoningLevel::Low),
        (ReasoningLevel::Medium, ReasoningLevel::Medium),
        (ReasoningLevel::High, ReasoningLevel::Medium),
        (ReasoningLevel::Max, ReasoningLevel::High),
    ];
    for (requested, expected) in cases {
        let fx = make_wired_sm("deepseek-v4-pro", requested, ProviderModelKnowledge::new()).await;
        let (sm, _caller) = (&fx.sm, fx.caller.clone());
        resolve_before_llm_call(sm, SESSION_ID).await;
        assert_effective(sm, expected).await;
    }
}

/// Missing session → resolution is a no-op (no panic).
#[tokio::test]
async fn test_boundary_session_not_found_no_panic() {
    let fx = make_wired_sm(
        "glm-5.1",
        ReasoningLevel::High,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, _caller) = (&fx.sm, fx.caller.clone());
    resolve_before_llm_call(sm, "nonexistent").await;
}

/// Gateway without model knowledge (no handler / no gateway ref) → no
/// write-back; the session keeps its fallback semantics (Step 1.1 contract).
#[tokio::test]
async fn test_boundary_no_knowledge_does_not_write_back() {
    // SessionManager without any gateway ref wired.
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    let mut cs = ConversationSession::new(SESSION_ID.to_string(), "glm-5.1".to_string(), workdir());
    cs.set_reasoning_level(ReasoningLevel::High);
    sm.conversation_sessions.write().await.insert(
        SESSION_ID.to_string(),
        Arc::new(tokio::sync::RwLock::new(cs)),
    );

    resolve_before_llm_call(&sm, SESSION_ID).await;

    assert_effective(&sm, ReasoningLevel::High).await;
}

// ── 长链场景: entries resolve BEFORE the invoke ──────────────────────────

/// Non-streaming entry: gateway resolve BEFORE `invoke_llm` — the captured
/// request must carry the effective (downgraded) level, not the requested one.
#[tokio::test]
async fn test_long_chain_non_streaming_resolve_before_invoke_carries_effective() {
    let fx = make_wired_sm(
        "glm-5.1",
        ReasoningLevel::Max,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, caller) = (&fx.sm, fx.caller.clone());

    // Entry-point order: resolve → invoke (mirrors drain_pending_loop).
    resolve_before_llm_call(sm, SESSION_ID).await;
    let cs = sm.get_conversation_session(SESSION_ID).await.unwrap();
    let response = cs.write().await.invoke_llm("hello").await;
    assert!(response.is_ok(), "invoke_llm should succeed");

    let req = caller.last_non_streaming().expect("request captured");
    assert_eq!(
        req.reasoning_level,
        ReasoningLevel::High,
        "request built after pre-call resolve carries the effective level"
    );
    assert_effective(sm, ReasoningLevel::High).await;
}

/// Streaming entry: gateway resolve BEFORE `invoke_llm_streaming` — the
/// captured stream request must carry the effective (downgraded) level.
#[tokio::test]
async fn test_long_chain_streaming_resolve_before_invoke_carries_effective() {
    let fx = make_wired_sm(
        "deepseek-v4-flash",
        ReasoningLevel::Max,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, caller) = (&fx.sm, fx.caller.clone());

    // Entry-point order: resolve → invoke streaming (mirrors call_llm_streaming).
    resolve_before_llm_call(sm, SESSION_ID).await;
    let cs = sm.get_conversation_session(SESSION_ID).await.unwrap();
    let stream = cs.write().await.invoke_llm_streaming("hello").await;
    assert!(stream.is_ok(), "invoke_llm_streaming should succeed");

    let req = caller.last_streaming().expect("stream request captured");
    assert_eq!(
        req.reasoning_level,
        ReasoningLevel::High,
        "streaming request built after pre-call resolve carries the effective level"
    );
    assert_effective(sm, ReasoningLevel::High).await;
}

/// Counterfactual: without the pre-call resolve the effective level stays
/// the un-downgraded requested value — proves the resolution is what moved
/// the session state (「前」语义反证）.
#[tokio::test]
async fn test_long_chain_without_resolve_effective_stays_requested() {
    let fx = make_wired_sm(
        "glm-5.1",
        ReasoningLevel::Max,
        ProviderModelKnowledge::new(),
    )
    .await;
    let (sm, caller) = (&fx.sm, fx.caller.clone());

    // Invoke WITHOUT the pre-call resolve.
    let cs = sm.get_conversation_session(SESSION_ID).await.unwrap();
    let response = cs.write().await.invoke_llm("hello").await;
    assert!(response.is_ok());

    let req = caller.last_non_streaming().expect("request captured");
    assert_eq!(
        req.reasoning_level,
        ReasoningLevel::Max,
        "without resolve the request carries the raw requested level"
    );
    assert_effective(sm, ReasoningLevel::Max).await;

    // Only the resolve moves the session to the effective level.
    resolve_before_llm_call(sm, SESSION_ID).await;
    assert_effective(sm, ReasoningLevel::High).await;
}
