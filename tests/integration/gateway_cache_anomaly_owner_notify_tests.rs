//! Integration tests: cache-anomaly Owner-directed notification through
//! the real queue → drain → route → Gateway simplified-path chain (issue
//! #3576 Step 1.2).
//!
//! Drives real session rounds (canned-usage caller with an explicit
//! cache-hit drop) so the anomaly is genuinely detected by the
//! session-side check, then applies the disposal block's public session
//! API (`check_cache_anomaly_and_advance` → OwnerOnly-declared push) —
//! the disposal block itself is gateway-internal; its exact production
//! behavior (OwnerOnly declaration + text shape) is pinned by the
//! gateway UT `session_handler_cache_check_tests.rs`. The checkpoint
//! `sender_id` (written by `find_or_create` from the message sender) is
//! the Gateway's current-user judgment input (outbound-flow「定向收件人」):
//!
//! - sender = "owner" + anomaly → the notification reaches the plugin
//!   with the full reference / hit / deviation text
//! - sender non-owner + anomaly → silently dropped (no delivery, no
//!   error, entry consumed)
//!
//! Step 1.3 additionally drives the *real* gateway dispatch path
//! (`SessionMessageHandler::handle_message_with_gateway` →
//! `finish_llm` → the internal disposal block) with a DebugLog wired on
//! the Gateway, asserting the structured
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
use closeclaw_common::{current_epoch_millis, NotificationPriority};
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogEvent, LogLevel};
use closeclaw_gateway::session_handler::{MessageMetadata, SessionMessageHandler};
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_gateway::{Gateway, GatewayConfig, Message};
use closeclaw_llm::cache_adapter::provider_cache_ttl_ms;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_session::llm_session::{ChatSession, ConversationSession, NotificationAudience};
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

/// One production-shaped round at the public session API: append the
/// user message → invoke (stages the request prefix snapshot) → append
/// the keepalive response → disposal + accumulation. The disposal block
/// mirrors `session_handler_cache_check.rs` (gateway-internal) via its
/// public session-API surface, including the OwnerOnly declaration.
async fn run_round(cs: &mut ConversationSession, content: &str) {
    cs.append_user_message(content);
    let (response, call_target) = cs.invoke_llm_with_target(content).await.unwrap();
    cs.append_response(response.clone());
    let call_model = call_target
        .as_ref()
        .map(|t| t.id_string())
        .unwrap_or_else(|| cs.model().to_string());
    let ttl_ms = provider_cache_ttl_ms(
        call_target
            .as_ref()
            .map(|t| t.provider_id.as_str())
            .unwrap_or_default(),
    );
    if let Some(info) = cs.check_cache_anomaly_and_advance(
        &response.usage,
        Some(&call_model),
        current_epoch_millis(),
        ttl_ms,
    ) {
        cs.push_system_notification_with_audience(
            info.format_notification(),
            NotificationPriority::Now,
            NotificationAudience::OwnerOnly,
        );
    }
    cs.accumulate_usage(&response.usage, Some(&call_model));
}

// ── Tests ────────────────────────────────────────────────────────────

/// sender = owner + real anomaly (round-2 cache hit drops 100000 → 0,
/// deviation 100000 > 1024 tolerance) → the notification reaches the
/// plugin with the full reference / hit / deviation text; the session
/// round itself completes normally (stats advanced).
#[tokio::test]
async fn test_owner_sender_receives_cache_anomaly_notification() {
    let (_tmp, sm, gw, delivered, sid, _recorder) = setup("owner").await;

    {
        let cs = sm.get_conversation_session(&sid).await.unwrap();
        let mut cs = cs.write().await;
        // Round 1: baseline — exempt (no previous snapshot).
        run_round(&mut cs, "hi").await;
        // Round 2: tail-append extension → participates; full miss.
        run_round(&mut cs, "hi").await;
        assert_eq!(
            cs.stats().request_count,
            2,
            "anomaly disposal must not block the round chain"
        );
    }

    sm.drain_and_inject_announces(&sid, Some(&gw)).await;

    let texts = delivered.lock().unwrap().clone();
    assert_eq!(
        texts.len(),
        1,
        "owner must receive exactly one notification, got: {texts:?}"
    );
    assert!(texts[0].starts_with("[缓存命中异常]"), "text: {}", texts[0]);
    assert!(texts[0].contains("参考值 100000"), "text: {}", texts[0]);
    assert!(texts[0].contains("本次命中 0"), "text: {}", texts[0]);
    assert!(texts[0].contains("偏差 100000"), "text: {}", texts[0]);
}

/// sender non-owner + real anomaly → silently dropped: no plugin
/// delivery, no error, and the queue entry is consumed (drop ≠
/// re-queue).
#[tokio::test]
async fn test_non_owner_sender_cache_anomaly_notification_dropped() {
    let (_tmp, sm, gw, delivered, sid, _recorder) = setup("user-a").await;

    {
        let cs = sm.get_conversation_session(&sid).await.unwrap();
        let mut cs = cs.write().await;
        run_round(&mut cs, "hi").await;
        run_round(&mut cs, "hi").await;
        assert_eq!(cs.stats().request_count, 2);
    }

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

/// Creates a `DebugLog` instance backed by a temp directory.
async fn make_debug_log(dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: dir.path().to_path_buf(),
        retention_days: 7,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.unwrap()
}

/// Reads all JSONL log events from the temp directory.
fn read_log_events(dir: &TempDir) -> Vec<LogEvent> {
    let mut events = Vec::new();
    for entry in std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().map(|ext| ext == "jsonl").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap();
            for line in content.lines().filter(|l| !l.trim().is_empty()) {
                events.push(LogEvent::from_jsonl(line).unwrap());
            }
        }
    }
    events
}

/// Bounded readiness poll (no fixed sleep): waits until at least one
/// `session.cache_anomaly.detected` event lands in the JSONL files, up
/// to 5s (two full dispatch rounds must complete first).
async fn wait_for_cache_anomaly_events(dir: &TempDir) -> Vec<LogEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let events: Vec<LogEvent> = read_log_events(dir)
            .into_iter()
            .filter(|e| e.event_type == "session.cache_anomaly.detected")
            .collect();
        if !events.is_empty() || tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

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

    // Handler with a chain-less unified fallback (the session-level
    // CannedUsageCaller set in `setup` is what the dispatch invokes).
    let cooldown_dir = TempDir::new().unwrap();
    let unified = Arc::new(UnifiedFallbackClient::new(
        vec![],
        Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        )),
    ));
    let handler = SessionMessageHandler::new_no_output(
        Arc::clone(&sm),
        unified,
        None,
        closeclaw_session::compaction::CompactConfig::default(),
    );

    let mut meta = MessageMetadata::default_meta();
    meta.sender_id = "owner".into();
    meta.channel = "tracking".into();

    // Round 1: baseline (exempt) → completes normally, no event.
    let result = handler
        .handle_message_with_gateway(&sid, "hi".into(), None, meta.clone(), &gw, &recorder)
        .await;
    assert!(matches!(
        result,
        closeclaw_gateway::session_handler::HandleResult::LlmStarted
    ));
    wait_for_round(&sm, &sid, 1).await;

    // Round 2: tail-append extension, full miss → anomaly.
    let result = handler
        .handle_message_with_gateway(&sid, "hi".into(), None, meta, &gw, &recorder)
        .await;
    assert!(matches!(
        result,
        closeclaw_gateway::session_handler::HandleResult::LlmStarted
    ));
    wait_for_round(&sm, &sid, 2).await;

    let events = wait_for_cache_anomaly_events(&debug_dir).await;
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
