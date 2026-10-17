//! Integration tests: cache-anomaly Owner-directed notification through
//! the real queue → drain → route → Gateway simplified-path chain (issue
//! #3576 Step 1.2).
//!
//! Drives the REAL gateway dispatch path
//! (`SessionMessageHandler::handle_message_with_gateway` → `finish_llm`
//! → the internal disposal block in `session_handler_cache_check.rs`)
//! with a canned-usage caller whose explicit cache-hit drop makes the
//! session-side check genuinely flag an anomaly, then drains the
//! announce queue through the real route — the Owner-receives /
//! non-Owner-drops assertions directly constrain production behavior.
//! The checkpoint `sender_id` (written by `find_or_create` from the
//! message sender) is the Gateway's current-user judgment input
//! (outbound-flow「定向收件人」):
//!
//! - sender = "owner" + anomaly → the notification reaches the plugin
//!   with the full reference / hit / deviation text
//! - sender non-owner + anomaly → silently dropped (no delivery, no
//!   error, entry consumed)
//!
//! Step 1.3 additionally wires a DebugLog on the Gateway for the same
//! dispatch path, asserting the structured
//! `session.cache_anomaly.detected` JSONL event (level Error,
//! `source_module="session"`, four-field payload, session_id
//! correlation).

#![cfg(feature = "fake-llm")]

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::llm_caller::{CallTarget, LlmCaller};
use closeclaw_common::llm_error::LLMError;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::processor::{
    ContentBlock, DslParseResult, StreamEvent, UnifiedResponse, UnifiedUsage,
};
use closeclaw_debug_log::test_utils::{make_debug_log, wait_for_event_type};
use closeclaw_debug_log::LogLevel;
use closeclaw_gateway::session_handler::{HandleResult, MessageMetadata, SessionMessageHandler};
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_gateway::{Gateway, GatewayConfig, Message};
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};
use futures::Stream;
use tempfile::TempDir;

// ── Fakes ────────────────────────────────────────────────────────────

/// Minimal in-memory persistence holding checkpoints (the checkpoint
/// `sender_id` is the Gateway's current-user judgment input).
struct MockPersist(tokio::sync::Mutex<HashMap<String, SessionCheckpoint>>);

impl MockPersist {
    fn new() -> Self {
        Self(tokio::sync::Mutex::new(HashMap::new()))
    }
}

#[async_trait]
impl PersistenceService for MockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.0
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.0.lock().await.get(sid).cloned())
    }
    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self.0.lock().await.keys().cloned().collect())
    }
}

/// Spy plugin (platform "tracking") recording every delivered text.
struct RecordingPlugin {
    messages: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl IMPlugin for RecordingPlugin {
    fn platform(&self) -> &str {
        "tracking"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({ "content": { "text": text } }),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_owned();
        if !text.is_empty() {
            self.messages.lock().unwrap().push(text);
        }
        Ok(())
    }
}

/// Canned-usage caller: each round's response carries the next
/// `UnifiedUsage` from the queue (the last entry repeats) and reports a
/// stable [`CallTarget`], mirroring the production fallback chain's
/// actually-served target. Keepalive-shaped empty content blocks keep
/// the rounds in a tail-append relationship (`append_response` treats
/// them as a documented no-op).
struct CannedUsageCaller {
    usages: Mutex<Vec<UnifiedUsage>>,
    target: CallTarget,
}

impl CannedUsageCaller {
    fn new(usages: Vec<UnifiedUsage>, provider: &str, model: &str) -> Self {
        Self {
            usages: Mutex::new(usages),
            target: CallTarget {
                provider_id: provider.to_string(),
                model_id: model.to_string(),
            },
        }
    }

    fn next_usage(&self) -> UnifiedUsage {
        let mut queue = self.usages.lock().unwrap();
        if queue.len() > 1 {
            queue.remove(0)
        } else {
            queue[0].clone()
        }
    }
}

fn response_with(usage: UnifiedUsage) -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![],
        usage,
        finish_reason: None,
        retry_attempts: 0,
    }
}

#[async_trait]
impl LlmCaller for CannedUsageCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Ok(response_with(self.next_usage()))
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>, LLMError> {
        Err(LLMError::InvalidRequest(
            "streaming not used in this test fake".to_string(),
        ))
    }

    async fn call_with_target(
        &self,
        _request: InternalRequest,
    ) -> Result<(UnifiedResponse, Option<CallTarget>), LLMError> {
        let usage = self.next_usage();
        Ok((response_with(usage), Some(self.target.clone())))
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn usage(prompt: u32, cache_read: Option<u32>) -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: prompt,
        completion_tokens: 200,
        total_tokens: Some(prompt + 200),
        reasoning_tokens: None,
        cache_read_tokens: cache_read,
        cache_write_tokens: None,
    }
}

/// Builds the in-process Gateway + SessionManager (checkpoint
/// `sender_id` = the message sender, per the `find_or_create` resolve
/// contract) with the recording plugin on the session channel.
#[allow(clippy::type_complexity)]
async fn setup(
    sender_id: &str,
) -> (
    TempDir,
    Arc<SessionManager>,
    Arc<Gateway>,
    Arc<Mutex<Vec<String>>>,
    String,
    Arc<dyn IMPlugin>,
) {
    let tmp = TempDir::new().unwrap();
    let config = GatewayConfig {
        name: "cache-anomaly-owner-notify-test".into(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(MockPersist::new()) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));

    let messages = Arc::new(Mutex::new(Vec::new()));
    let gw = Arc::new(Gateway::new_for_tests(config, Arc::clone(&sm)));
    let plugin: Arc<dyn IMPlugin> = Arc::new(RecordingPlugin {
        messages: Arc::clone(&messages),
    });
    gw.register_plugin(Arc::clone(&plugin)).await;

    let msg = Message {
        id: "msg_1".into(),
        from: sender_id.to_string(),
        to: "chat-cache-anomaly".into(),
        content: "hi".into(),
        channel: "tracking".into(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    // Controllable caller on the session (anomaly shaped by the caller's
    // canned usages, reported target = "fake/cache-model").
    let cs = sm.get_conversation_session(&sid).await.unwrap();
    cs.write()
        .await
        .set_llm_caller(Arc::new(CannedUsageCaller::new(
            vec![usage(100_000, Some(100_000)), usage(100_000, Some(0))],
            "fake",
            "cache-model",
        )));

    (tmp, sm, gw, messages, sid, plugin)
}

/// Chain-less unified fallback handler for the real-dispatch-path tests:
/// the session-level `CannedUsageCaller` installed by `setup` is what
/// the dispatch actually invokes. The `TempDir` backing the cooldown
/// file path is returned so it outlives the handler.
fn dispatch_handler(sm: &Arc<SessionManager>) -> (SessionMessageHandler, TempDir) {
    let cooldown_dir = TempDir::new().unwrap();
    let unified = Arc::new(UnifiedFallbackClient::new(
        vec![],
        Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        )),
    ));
    (
        SessionMessageHandler::new_no_output(
            Arc::clone(sm),
            unified,
            None,
            closeclaw_session::compaction::CompactConfig::default(),
        ),
        cooldown_dir,
    )
}

/// Drives two REAL dispatch rounds through the gateway path
/// (`handle_message_with_gateway` → `finish_llm` → the internal disposal
/// block): round 1 is the exempt baseline, round 2 is the tail-append
/// full miss that triggers the anomaly disposal. Each round must
/// dispatch to the LLM and complete with the disposal block's
/// accumulation advanced — the round chain must not be blocked.
async fn run_two_dispatch_rounds(
    sm: &Arc<SessionManager>,
    gw: &Arc<Gateway>,
    handler: &SessionMessageHandler,
    sid: &str,
    sender_id: &str,
    plugin: &Arc<dyn IMPlugin>,
) {
    let mut meta = MessageMetadata::default_meta();
    meta.sender_id = sender_id.to_string();
    meta.channel = "tracking".into();
    for n in 1..=2 {
        let result = handler
            .handle_message_with_gateway(sid, "hi".into(), None, meta.clone(), gw, plugin)
            .await;
        assert!(
            matches!(result, HandleResult::LlmStarted),
            "round {n} should dispatch to the LLM"
        );
        wait_for_round(sm, sid, n).await;
    }
}

// ── Tests ────────────────────────────────────────────────────────────

/// sender = owner + real anomaly (round-2 cache hit drops 100000 → 0,
/// deviation 100000 > 1024 tolerance) through the REAL gateway dispatch
/// path → the notification reaches the plugin with the full reference /
/// hit / deviation text; the session round itself completes normally
/// (dispatch rounds finish — disposal never blocks the round chain).
///
/// The turn-end internal drain (`drain_announces_rest`) races the test's
/// own `drain_and_inject_announces` sweep: whoever drains first delivers
/// (the internal one prefixes `[紧急] `, the unfiltered sweep does not),
/// so the text assertions match content, not the priority prefix — the
/// sweep guarantees the entry is delivered exactly once either way.
#[tokio::test]
async fn test_owner_sender_receives_cache_anomaly_notification() {
    let (_tmp, sm, gw, delivered, sid, recorder) = setup("owner").await;
    let (handler, _cooldown_dir) = dispatch_handler(&sm);

    run_two_dispatch_rounds(&sm, &gw, &handler, &sid, "owner", &recorder).await;

    sm.drain_and_inject_announces(&sid, Some(&gw)).await;

    let texts = delivered.lock().unwrap().clone();
    assert_eq!(
        texts.len(),
        1,
        "owner must receive exactly one notification, got: {texts:?}"
    );
    assert!(texts[0].contains("[缓存命中异常]"), "text: {}", texts[0]);
    assert!(texts[0].contains("参考值 100000"), "text: {}", texts[0]);
    assert!(texts[0].contains("本次命中 0"), "text: {}", texts[0]);
    assert!(texts[0].contains("偏差 100000"), "text: {}", texts[0]);
}

/// sender non-owner + real anomaly through the REAL gateway dispatch
/// path → silently dropped: no plugin delivery, no error, and the queue
/// entry is consumed (drop ≠ re-queue).
#[tokio::test]
async fn test_non_owner_sender_cache_anomaly_notification_dropped() {
    let (_tmp, sm, gw, delivered, sid, recorder) = setup("user-a").await;
    let (handler, _cooldown_dir) = dispatch_handler(&sm);

    run_two_dispatch_rounds(&sm, &gw, &handler, &sid, "user-a", &recorder).await;

    sm.drain_and_inject_announces(&sid, Some(&gw)).await;

    assert!(
        delivered.lock().unwrap().is_empty(),
        "non-owner must not receive the cache-anomaly notification"
    );
    assert!(
        sm.get_conversation_session(&sid)
            .await
            .unwrap()
            .read()
            .await
            .is_queue_empty(),
        "dropped entry must be consumed, not re-queued"
    );
}

// ── Step 1.3: debug_log event through the real gateway path ─────────

/// Drives the REAL gateway dispatch path
/// (`handle_message_with_gateway` → `finish_llm` → internal disposal
/// block) for two rounds with a DebugLog wired on the Gateway: the
/// second round's full cache miss (100000 → 0, deviation > 1024) must
/// surface as a `session.cache_anomaly.detected` JSONL event with
/// level Error、`source_module="session"`、session_id correlation and
/// the four `CacheAnomalyInfo` payload fields — while the round chain
/// itself completes normally (issue #3576 Step 1.3).
#[tokio::test]
async fn test_real_gateway_path_emits_cache_anomaly_debug_log_event() {
    let (_tmp, sm, gw, _delivered, sid, recorder) = setup("owner").await;
    let debug_dir = TempDir::new().unwrap();
    gw.set_debug_log(make_debug_log(&debug_dir).await).await;
    let (handler, _cooldown_dir) = dispatch_handler(&sm);

    run_two_dispatch_rounds(&sm, &gw, &handler, &sid, "owner", &recorder).await;

    let events = wait_for_event_type(debug_dir.path(), "session.cache_anomaly.detected").await;
    assert_eq!(
        events.len(),
        1,
        "exactly one anomaly event expected, got: {events:?}"
    );
    let evt = &events[0];
    assert_eq!(evt.level, LogLevel::Error);
    assert_eq!(evt.source_module, "session");
    assert_eq!(evt.event_type, "session.cache_anomaly.detected");
    // session_id as the correlation key.
    assert_eq!(evt.trace_id, sid);
    assert_eq!(evt.session_key.as_deref(), Some(sid.as_str()));
    assert_eq!(evt.payload["previous_prompt_tokens"], 100_000);
    assert_eq!(evt.payload["current_cache_read_tokens"], 0);
    assert_eq!(evt.payload["deviation_tokens"], 100_000);
    assert_eq!(evt.payload["tolerance_tokens"], 1024);
}

/// Bounded readiness poll for the dispatch task finishing a round:
/// `request_count` (advanced by the disposal block's accumulation)
/// reaches `n` and the session is idle again.
async fn wait_for_round(sm: &Arc<SessionManager>, sid: &str, n: u64) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let done = match sm.get_conversation_session(sid).await {
            Some(cs) => {
                let cs = cs.read().await;
                cs.stats().request_count == n && !cs.is_llm_busy()
            }
            None => false,
        };
        if done || tokio::time::Instant::now() >= deadline {
            assert!(done, "round {n} did not complete in time");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
