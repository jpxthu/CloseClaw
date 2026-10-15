//! Gateway-side tests for the extracted unified cache-check function
//! ([`check_cache_anomaly_and_accumulate_usage`]).
//!
//! Dimensions (issue #3620 Step 1.4 ③):
//! - participating round with deviation beyond tolerance → metric
//!   emission + Owner system notification
//! - participating round within tolerance → no emission, no notification
//! - exempt rounds (first round / model switch) → no emission, no
//!   notification; usage still accumulated
//! - no reported call target → degrade to the session-config model
//!   identifier without panicking (fake/stub paths)
//!
//! Round construction note: the fake serves keepalive responses (empty
//! content blocks — `ChatSession::append_response` treats them as a
//! documented no-op) so the round's sent request lists stay in a
//! tail-append relationship, which is the participating-round shape.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use closeclaw_common::{CacheAnomalyInfo, CallTarget, LLMError, LlmCaller, MetricsEmitter};
use closeclaw_llm::types::{InternalRequest, UnifiedResponse, UnifiedUsage};
use closeclaw_session::llm_session::{ChatSession, ConversationSession, QueueEntry};

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
) {
    cs.append_user_message(user_content);
    let (response, call_target) = cs.invoke_llm_with_target(user_content).await.unwrap();
    cs.append_response(response.clone());
    let mut stream_result = StreamResult::from(response);
    stream_result.call_target = call_target;
    stream_result.usage = usage;
    check_cache_anomaly_and_accumulate_usage("sess-cache-check", cs, &stream_result, emitter);
}

/// Drains the unified queue and returns the system notification texts.
fn drained_system_notifications(cs: &mut ConversationSession) -> Vec<String> {
    cs.drain_queue()
        .into_iter()
        .filter_map(|entry| match entry {
            QueueEntry::SystemNotification(text, _, _) => Some(text),
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

// ── Tests ────────────────────────────────────────────────────────────

/// Participating round with a cache-read drop beyond the fixed 1024
/// tolerance → metric emission + Owner system notification; usage is
/// still accumulated with the actually-reported model identifier.
#[tokio::test]
async fn test_anomaly_emits_metric_and_owner_notification() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cs = make_session(&tmp, vec![Some(target("prov-a", "model-a"))]);
    let emitter = Arc::new(CountingEmitter::default());
    let em: Option<Arc<dyn MetricsEmitter>> = Some(emitter.clone());

    // Round 1: baseline — exempt (no previous snapshot), usage accumulates.
    run_round(&mut cs, "hi", make_usage(100_000, Some(100_000)), &em).await;
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
    run_round(&mut cs, "hi", make_usage(100_000, Some(0)), &em).await;
    assert_eq!(emitter.count.load(Ordering::SeqCst), 1);
    let last = emitter.last.lock().unwrap().clone().unwrap();
    assert_eq!(last.previous_prompt_tokens, 100_000);
    assert_eq!(last.current_cache_read_tokens, 0);
    assert_eq!(last.tolerance_tokens, 1024);
    let notifications = drained_system_notifications(&mut cs);
    assert_eq!(notifications.len(), 1, "Owner notification must be pushed");
    assert!(notifications[0].starts_with("[缓存命中异常]"));
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

    run_round(&mut cs, "hi", make_usage(100_000, Some(100_000)), &em).await;
    run_round(&mut cs, "hi", make_usage(100_000, Some(99_500)), &em).await;

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

    run_round(&mut cs, "hi", make_usage(100_000, Some(100_000)), &em).await;
    // Round 2: same prefix but a different reported target → the round
    // is exempt from the numeric comparison despite the full miss.
    run_round(&mut cs, "hi", make_usage(100_000, Some(0)), &em).await;

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

    run_round(&mut cs, "hi", make_usage(100_000, Some(100_000)), &em).await;

    assert_eq!(emitter.count.load(Ordering::SeqCst), 0);
    assert!(drained_system_notifications(&mut cs).is_empty());
    assert_eq!(
        cs.stats().last_call_model.as_deref(),
        Some("cfg-model"),
        "no target must degrade to the session-config model"
    );
    assert_eq!(cs.stats().request_count, 1);
}
