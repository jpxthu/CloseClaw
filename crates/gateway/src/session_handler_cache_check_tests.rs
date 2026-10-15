//! Gateway-side tests for the extracted unified cache-check function
//! ([`check_cache_anomaly_and_accumulate_usage`]).
//!
//! Dimensions (issue #3620 Step 1.4 ③ / issue #3576 Step 1.2 + 1.3):
//! - participating round with deviation beyond tolerance → metric
//!   emission + Owner-directed system notification (declared
//!   [`NotificationAudience::OwnerOnly`])
//! - anomaly round with a `DebugLog` instance → structured
//!   `session.cache_anomaly.detected` event written to JSONL (level
//!   Error, `source_module="session"`, four-field payload, session_id
//!   correlation)
//! - participating round within tolerance → no emission, no notification
//! - exempt rounds (first round / model switch) → no emission, no
//!   notification; usage still accumulated
//! - no reported call target → degrade to the session-config model
//!   identifier without panicking (fake/stub paths)
//! - no `DebugLog` instance → no panic, disposal chain unaffected
//!   (every `None`-debug_log test below exercises this)
//!
//! Round construction note: the fake serves keepalive responses (empty
//! content blocks — `ChatSession::append_response` treats them as a
//! documented no-op) so the round's sent request lists stay in a
//! tail-append relationship, which is the participating-round shape.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use closeclaw_common::{CacheAnomalyInfo, CallTarget, LLMError, LlmCaller, MetricsEmitter};
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogEvent, LogLevel};
use closeclaw_llm::types::{InternalRequest, UnifiedResponse, UnifiedUsage};
use closeclaw_session::llm_session::{
    ChatSession, ConversationSession, NotificationAudience, QueueEntry,
};

use crate::outbound::StreamResult;
use crate::session_handler_cache_check::check_cache_anomaly_and_accumulate_usage;

// ── Fakes ────────────────────────────────────────────────────────────

/// Canned-keepalive caller whose per-call targets come from a queue
/// (the last entry repeats). Mirrors the production flow where the
/// fallback chain reports the actually-served (provider, model).
struct TargetedFakeCaller {
    targets: Mutex<Vec<Option<CallTarget>>>,
}

impl TargetedFakeCaller {
    fn new(targets: Vec<Option<CallTarget>>) -> Self {
        Self {
            targets: Mutex::new(targets),
        }
    }
}

#[async_trait::async_trait]
impl LlmCaller for TargetedFakeCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        Ok(keepalive_response())
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures::Stream<
                        Item = Result<closeclaw_common::processor::StreamEvent, LLMError>,
                    > + Send,
            >,
        >,
        LLMError,
    > {
        Err(LLMError::InvalidRequest(
            "streaming not supported in test fake".to_string(),
        ))
    }

    async fn call_with_target(
        &self,
        request: InternalRequest,
    ) -> Result<(UnifiedResponse, Option<CallTarget>), LLMError> {
        let response = self.call(request).await?;
        let mut targets = self.targets.lock().unwrap();
        let target = if targets.len() > 1 {
            targets.remove(0)
        } else {
            targets[0].clone()
        };
        Ok((response, target))
    }
}

#[derive(Default)]
struct CountingEmitter {
    count: AtomicUsize,
    last: Mutex<Option<CacheAnomalyInfo>>,
}

impl MetricsEmitter for CountingEmitter {
    fn emit_cache_anomaly(&self, info: &CacheAnomalyInfo) {
        self.count.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some(info.clone());
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn make_usage(prompt: u32, cache_read: Option<u32>) -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: prompt,
        completion_tokens: 200,
        total_tokens: Some(prompt + 200),
        reasoning_tokens: None,
        cache_read_tokens: cache_read,
        cache_write_tokens: None,
    }
}

/// Keepalive-shaped response: empty content blocks make
/// `append_response` a documented no-op (no history mutation).
fn keepalive_response() -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![],
        usage: make_usage(1_000, None),
        finish_reason: None,
        retry_attempts: 0,
    }
}

fn make_session(tmp: &tempfile::TempDir, targets: Vec<Option<CallTarget>>) -> ConversationSession {
    let mut cs = ConversationSession::new(
        "sess-cache-check".into(),
        "cfg-model".into(),
        tmp.path().to_path_buf(),
    );
    cs.set_llm_caller(Arc::new(TargetedFakeCaller::new(targets)));
    cs
}

/// One production-shaped round: append the user message → invoke
/// (stages the request prefix snapshot) → append the keepalive
/// response (no-op) → attach the reported target and the round's
/// usage to the `StreamResult` → extracted check + accumulate
/// function.
async fn run_round(
    cs: &mut ConversationSession,
    user_content: &str,
    usage: UnifiedUsage,
    emitter: &Option<Arc<dyn MetricsEmitter>>,
    debug_log: Option<&DebugLog>,
) {
    cs.append_user_message(user_content);
    let (response, call_target) = cs.invoke_llm_with_target(user_content).await.unwrap();
    cs.append_response(response.clone());
    let mut stream_result = StreamResult::from(response);
    stream_result.call_target = call_target;
    stream_result.usage = usage;
    check_cache_anomaly_and_accumulate_usage(
        "sess-cache-check",
        cs,
        &stream_result,
        emitter,
        debug_log,
    );
}

/// Drains the unified queue and returns the system notifications as
/// `(text, audience)` pairs — the audience assertion pins the
/// Owner-directed declaration against semantic drift.
fn drained_system_notifications(
    cs: &mut ConversationSession,
) -> Vec<(String, NotificationAudience)> {
    cs.drain_queue()
        .into_iter()
        .filter_map(|entry| match entry {
            QueueEntry::SystemNotification(text, _, audience) => Some((text, audience)),
            _ => None,
        })
        .collect()
}

fn target(provider: &str, model: &str) -> CallTarget {
    CallTarget {
        provider_id: provider.to_string(),
        model_id: model.to_string(),
    }
}

/// Creates a `DebugLog` instance backed by a temp directory (trace
/// min-level so every emitted event is persisted).
async fn make_debug_log(dir: &tempfile::TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: dir.path().to_path_buf(),
        retention_days: 7,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.unwrap()
}

/// Reads all JSONL log events from the temp directory.
fn read_log_events(dir: &tempfile::TempDir) -> Vec<LogEvent> {
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
/// to 2s.
async fn wait_for_cache_anomaly_events(dir: &tempfile::TempDir) -> Vec<LogEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
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

// ── Tests ────────────────────────────────────────────────────────────

/// Participating round with a cache-read drop beyond the fixed 1024
/// tolerance → metric emission + Owner-directed system notification
/// (queued with the `NotificationAudience::OwnerOnly` declaration);
/// usage is still accumulated with the actually-reported model
/// identifier.
#[tokio::test]
async fn test_anomaly_emits_metric_and_owner_notification() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cs = make_session(&tmp, vec![Some(target("prov-a", "model-a"))]);
    let emitter = Arc::new(CountingEmitter::default());
    let em: Option<Arc<dyn MetricsEmitter>> = Some(emitter.clone());

    // Round 1: baseline — exempt (no previous snapshot), usage accumulates.
    run_round(&mut cs, "hi", make_usage(100_000, Some(100_000)), &em, None).await;
    assert_eq!(emitter.count.load(Ordering::SeqCst), 0);
    assert!(drained_system_notifications(&mut cs).is_empty());
    assert_eq!(cs.stats().last_prompt_tokens, Some(100_000));
    assert_eq!(
        cs.stats().last_call_model.as_deref(),
        Some("prov-a/model-a"),
        "accumulate must receive the actually-reported call target"
    );

    // Round 2: tail-append extension, same target, within TTL →
    // participates; cache read drops to 0 → deviation 100_000 > 1024.
    run_round(&mut cs, "hi", make_usage(100_000, Some(0)), &em, None).await;
    assert_eq!(emitter.count.load(Ordering::SeqCst), 1);
    let last = emitter.last.lock().unwrap().clone().unwrap();
    assert_eq!(last.previous_prompt_tokens, 100_000);
    assert_eq!(last.current_cache_read_tokens, 0);
    assert_eq!(last.tolerance_tokens, 1024);
    let notifications = drained_system_notifications(&mut cs);
    assert_eq!(notifications.len(), 1, "Owner notification must be pushed");
    assert!(
        notifications[0].0.starts_with("[缓存命中异常]"),
        "text: {}",
        notifications[0].0
    );
    assert_eq!(
        notifications[0].1,
        NotificationAudience::OwnerOnly,
        "cache-anomaly notification must declare the OwnerOnly audience"
    );
    assert_eq!(cs.stats().request_count, 2);
}

/// Participating round within the tolerance → no metric emission and
/// no notification.
#[tokio::test]
async fn test_participating_within_tolerance_no_emission() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cs = make_session(&tmp, vec![Some(target("prov-a", "model-a"))]);
    let emitter = Arc::new(CountingEmitter::default());
    let em: Option<Arc<dyn MetricsEmitter>> = Some(emitter.clone());

    run_round(&mut cs, "hi", make_usage(100_000, Some(100_000)), &em, None).await;
    run_round(&mut cs, "hi", make_usage(100_000, Some(99_500)), &em, None).await;

    assert_eq!(
        emitter.count.load(Ordering::SeqCst),
        0,
        "deviation 500 ≤ 1024 is within tolerance"
    );
    assert!(drained_system_notifications(&mut cs).is_empty());
    assert_eq!(cs.stats().request_count, 2);
}

/// Model switch on the second round → exempt even though the usage
/// would deviate; no emission, no notification; usage still
/// accumulated with the new round's target.
#[tokio::test]
async fn test_model_switch_round_exempt_no_emission() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cs = make_session(
        &tmp,
        vec![
            Some(target("prov-a", "model-a")),
            Some(target("prov-b", "model-b")),
        ],
    );
    let emitter = Arc::new(CountingEmitter::default());
    let em: Option<Arc<dyn MetricsEmitter>> = Some(emitter.clone());

    run_round(&mut cs, "hi", make_usage(100_000, Some(100_000)), &em, None).await;
    // Round 2: same prefix but a different reported target → the round
    // is exempt from the numeric comparison despite the full miss.
    run_round(&mut cs, "hi", make_usage(100_000, Some(0)), &em, None).await;

    assert_eq!(
        emitter.count.load(Ordering::SeqCst),
        0,
        "model-switch round is exempt"
    );
    assert!(drained_system_notifications(&mut cs).is_empty());
    assert_eq!(
        cs.stats().last_call_model.as_deref(),
        Some("prov-b/model-b"),
        "exempt rounds still advance the accumulated state"
    );
    assert_eq!(cs.stats().request_count, 2);
}

/// No reported call target (fake/stub paths) → degrades to the
/// session-config model identifier, no panic, exemption honored.
#[tokio::test]
async fn test_no_reported_target_degrades_to_session_config_model() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cs = make_session(&tmp, vec![None]);
    let emitter = Arc::new(CountingEmitter::default());
    let em: Option<Arc<dyn MetricsEmitter>> = Some(emitter.clone());

    run_round(&mut cs, "hi", make_usage(100_000, Some(100_000)), &em, None).await;

    assert_eq!(emitter.count.load(Ordering::SeqCst), 0);
    assert!(drained_system_notifications(&mut cs).is_empty());
    assert_eq!(
        cs.stats().last_call_model.as_deref(),
        Some("cfg-model"),
        "no target must degrade to the session-config model"
    );
    assert_eq!(cs.stats().request_count, 1);
}

/// Anomaly round with a `DebugLog` instance → a structured
/// `session.cache_anomaly.detected` event lands in the JSONL files:
/// level Error（「异常」级）、`source_module="session"`、session_id
/// correlation（trace_id + session_key）、payload carrying the four
/// `CacheAnomalyInfo` fields (issue #3576 Step 1.3).
#[tokio::test]
async fn test_anomaly_emits_debug_log_event() {
    let tmp = tempfile::TempDir::new().unwrap();
    let debug_dir = tempfile::TempDir::new().unwrap();
    let debug_log = make_debug_log(&debug_dir).await;
    let mut cs = make_session(&tmp, vec![Some(target("prov-a", "model-a"))]);
    let em: Option<Arc<dyn MetricsEmitter>> = None;

    // Round 1: baseline — exempt; round 2: full miss beyond tolerance.
    run_round(
        &mut cs,
        "hi",
        make_usage(100_000, Some(100_000)),
        &em,
        Some(&debug_log),
    )
    .await;
    run_round(
        &mut cs,
        "hi",
        make_usage(100_000, Some(0)),
        &em,
        Some(&debug_log),
    )
    .await;

    let events = wait_for_cache_anomaly_events(&debug_dir).await;
    assert_eq!(
        events.len(),
        1,
        "exactly one anomaly event expected, got: {events:?}"
    );
    let evt = &events[0];
    assert_eq!(evt.level, LogLevel::Error, "cache anomaly is「异常」级");
    assert_eq!(evt.source_module, "session");
    assert_eq!(evt.event_type, "session.cache_anomaly.detected");
    // session_id as the correlation key (no inbound trace metadata at
    // the advancement point).
    assert_eq!(evt.trace_id, "sess-cache-check");
    assert_eq!(evt.session_key.as_deref(), Some("sess-cache-check"));
    assert_eq!(evt.payload["previous_prompt_tokens"], 100_000);
    assert_eq!(evt.payload["current_cache_read_tokens"], 0);
    assert_eq!(evt.payload["deviation_tokens"], 100_000);
    assert_eq!(evt.payload["tolerance_tokens"], 1024);
}

/// Non-anomaly rounds with a `DebugLog` instance → no anomaly event
/// written (exempt / within-tolerance rounds stay silent in JSONL).
#[tokio::test]
async fn test_non_anomaly_rounds_write_no_debug_log_event() {
    let tmp = tempfile::TempDir::new().unwrap();
    let debug_dir = tempfile::TempDir::new().unwrap();
    let debug_log = make_debug_log(&debug_dir).await;
    let mut cs = make_session(&tmp, vec![Some(target("prov-a", "model-a"))]);
    let em: Option<Arc<dyn MetricsEmitter>> = None;

    // Round 1: exempt baseline; round 2: within tolerance.
    run_round(
        &mut cs,
        "hi",
        make_usage(100_000, Some(100_000)),
        &em,
        Some(&debug_log),
    )
    .await;
    run_round(
        &mut cs,
        "hi",
        make_usage(100_000, Some(99_500)),
        &em,
        Some(&debug_log),
    )
    .await;
    assert_eq!(cs.stats().request_count, 2);

    // Allow any (unexpected) spawned writes to settle before asserting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        read_log_events(&debug_dir).is_empty(),
        "no debug_log event expected without an anomaly"
    );
}
