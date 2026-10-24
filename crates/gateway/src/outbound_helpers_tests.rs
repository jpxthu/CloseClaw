//! Unit tests for outbound_helpers — streaming text dispatch.
//!
//! After Step 1.2, middleware is no longer applied per-chunk during
//! streaming. `send_text` and `send_render_block` send directly via
//! the plugin without middleware. Middleware gating is handled by
//! pre-flight check in `send_outbound_streaming_inner`.
//!
//! These tests verify the updated send_text behavior (no middleware).
//!
//! Log-capture note: tests asserting WARN output use the shared
//! `closeclaw_test_support::log_capture` entries and carry
//! `#[serial_test::serial]`. The tracing callsite-interest cache is
//! process-global; concurrent registration on the same callsite can
//! drop events and empty the capture buffer (issue #3102 race).

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::OutboundMiddleware;
use closeclaw_common::{ContentBlock, MiddlewareContext, MiddlewareError};

use super::inbound_queue::InboundRequest;
use super::inbound_queue_test_utils::queued;
use crate::outbound_helpers::{
    deliver_batch_result, send_simplified_with_timeout, send_text, StreamContext,
};
use crate::session_manager::test_helpers::make_msg;
use crate::{Gateway, GatewayConfig, SessionManager};
use closeclaw_session::llm_session::ChatSession;
use closeclaw_test_support::log_capture::capture_logs_async;

// ---------------------------------------------------------------------------
// Test helpers

fn test_gw() -> Gateway {
    let config = GatewayConfig {
        name: "outbound_helpers_test".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    };
    let sm = std::sync::Arc::new(SessionManager::new(&config, None, None, Default::default()));
    Gateway::new_for_tests(config, sm)
}

/// Render content blocks as a plain-text `RenderedOutput` — the shared body
/// behind every mock plugin's `render()` in this file (each impl delegates
/// here instead of repeating the join).
fn render_plain(content_blocks: &[ContentBlock]) -> RenderedOutput {
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
        payload: serde_json::json!({"content": {"text": text}}),
    }
}

// ---------------------------------------------------------------------------
// Mock plugin
// ---------------------------------------------------------------------------

/// Mock plugin that records every `send()` call for assertion.
struct SendTrackingPlugin {
    send_called: Arc<std::sync::atomic::AtomicBool>,
    last_text: Arc<std::sync::Mutex<Option<String>>>,
}

impl SendTrackingPlugin {
    fn new() -> (Self, SendTracker) {
        let send_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let last_text = Arc::new(std::sync::Mutex::new(None));
        let tracker = SendTracker {
            send_called: Arc::clone(&send_called),
            last_text: Arc::clone(&last_text),
        };
        (
            Self {
                send_called,
                last_text,
            },
            tracker,
        )
    }
}

/// Lightweight handle to query the tracking plugin's state.
struct SendTracker {
    send_called: Arc<std::sync::atomic::AtomicBool>,
    last_text: Arc<std::sync::Mutex<Option<String>>>,
}

impl SendTracker {
    fn was_send_called(&self) -> bool {
        self.send_called.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn last_sent_text(&self) -> Option<String> {
        self.last_text.lock().unwrap().clone()
    }
}

/// Assert that `send()` was dispatched for `scenario`.
///
/// Failure message follows STANDARDS §9 "expected / actual": the scenario,
/// the expected `send_called = true`, and the raw tracker fields.
fn assert_send_dispatched(tracker: &SendTracker, scenario: &str) {
    let send_called = tracker.was_send_called();
    assert!(
        send_called,
        "{}: expected send_called = true, but send_called = {}, last_sent_text = {:?}",
        scenario,
        send_called,
        tracker.last_sent_text()
    );
}

#[async_trait]
impl IMPlugin for SendTrackingPlugin {
    fn platform(&self) -> &str {
        "mock"
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
        render_plain(content_blocks)
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.send_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        *self.last_text.lock().unwrap() = Some(text);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock middleware (for reference / future use)
// ---------------------------------------------------------------------------

/// Middleware that always allows (returns Ok).
#[allow(dead_code)]
struct AllowMiddleware;

#[async_trait]
impl OutboundMiddleware for AllowMiddleware {
    fn name(&self) -> &str {
        "allow"
    }

    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_plugin() -> (Arc<dyn IMPlugin>, SendTracker) {
    let (plugin, tracker) = SendTrackingPlugin::new();
    (Arc::new(plugin), tracker)
}

fn make_stream_ctx<'a>(
    plugin: &'a Arc<dyn IMPlugin>,
    session_id: &'a str,
    channel: &'a str,
    chat_id: &'a str,
    gateway: &'a Gateway,
) -> StreamContext<'a> {
    StreamContext {
        gateway,
        plugin,
        session_id,
        channel,
        chat_id,
        thread_id: None,
        reply_ref: None,
        registry: None,
        trace_id: None,
        session_key: None,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

/// send_text dispatches text directly via plugin.send (no middleware).
#[tokio::test]
async fn test_send_text_dispatches_directly() {
    let (plugin, tracker) = make_plugin();
    let gw = test_gw();
    let ctx = make_stream_ctx(&plugin, "s1", "mock", "chat1", &gw);
    send_text(&ctx, "hello world").await.unwrap();
    assert_send_dispatched(&tracker, "send_text(\"hello world\")");
    assert_eq!(tracker.last_sent_text().unwrap(), "hello world");
}

/// send_text with empty text still dispatches.
#[tokio::test]
async fn test_send_text_empty_string() {
    let (plugin, tracker) = make_plugin();
    let gw = test_gw();
    let ctx = make_stream_ctx(&plugin, "s2", "mock", "chat2", &gw);
    send_text(&ctx, "").await.unwrap();
    assert_send_dispatched(&tracker, "send_text(\"\") (empty string)");
    assert_eq!(tracker.last_sent_text().unwrap(), "");
}

/// send_text with special characters dispatches correctly.
#[tokio::test]
async fn test_send_text_special_characters() {
    let (plugin, tracker) = make_plugin();
    let gw = test_gw();
    let ctx = make_stream_ctx(&plugin, "s3", "mock", "chat3", &gw);
    send_text(&ctx, "hello 🌍 <script>alert('xss')</script>")
        .await
        .unwrap();
    assert_send_dispatched(
        &tracker,
        "send_text(\"hello 🌍 <script>alert('xss')</script>\") (special characters)",
    );
    assert_eq!(
        tracker.last_sent_text().unwrap(),
        "hello 🌍 <script>alert('xss')</script>"
    );
}

// ===========================================================================
// send_simplified_with_timeout tests (Step 1.4)
// ===========================================================================

/// Mock plugin whose `send()` completes immediately.
struct FastSendPlugin;

#[async_trait]
impl IMPlugin for FastSendPlugin {
    fn platform(&self) -> &str {
        "mock"
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
        render_plain(content_blocks)
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// Mock plugin whose `send()` sleeps for 3 seconds (exceeds 2s timeout).
///
/// The sleep is a `tokio::time::sleep`, so on the paused clock of
/// [`test_send_simplified_with_timeout_timeout_path`] it is advanced
/// virtually — no wall-clock cost.
struct SlowSendPlugin;

#[async_trait]
impl IMPlugin for SlowSendPlugin {
    fn platform(&self) -> &str {
        "mock"
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
        render_plain(content_blocks)
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        Ok(())
    }
}

/// send_simplified_with_timeout returns Ok when plugin.send() completes quickly.
#[tokio::test]
async fn test_send_simplified_with_timeout_normal_path() {
    let gw = test_gw();
    gw.register_plugin(Arc::new(FastSendPlugin) as Arc<dyn IMPlugin>)
        .await;
    let result = send_simplified_with_timeout(&gw, "chat1", "mock", "hello").await;
    assert!(
        result.is_ok(),
        "normal path should return Ok, got {:?}",
        result
    );
}

/// send_simplified_with_timeout returns Ok (drops message) when plugin.send()
/// takes longer than 2 seconds — the 2s deadline fires first and the
/// in-flight send is abandoned.
///
/// Runs on a paused tokio clock (`start_paused = true`): both the production
/// `tokio::time::timeout(2s)` and the mock's 3s `tokio::time::sleep` register
/// on the virtual clock, and tokio auto-advance jumps exactly to the next
/// timer deadline. The elapsed assertion pins the call to the exact 2s
/// virtual deadline — a completed send would read 3s, a mis-wired clock ~0 —
/// while wall-clock cost collapses to milliseconds.
#[tokio::test(start_paused = true)]
async fn test_send_simplified_with_timeout_timeout_path() {
    let gw = test_gw();
    gw.register_plugin(Arc::new(SlowSendPlugin) as Arc<dyn IMPlugin>)
        .await;
    let start = tokio::time::Instant::now();
    let result = send_simplified_with_timeout(&gw, "chat2", "mock", "slow-msg").await;
    let elapsed = start.elapsed();
    assert!(
        result.is_ok(),
        "timeout path should return Ok, got {:?}",
        result
    );
    assert_eq!(
        elapsed,
        std::time::Duration::from_secs(2),
        "timeout must resolve at the exact 2s virtual deadline (send sleeps 3s), got {:?}",
        elapsed
    );
}

/// send_simplified_with_timeout returns Ok when plugin is not registered
/// (falls back to plain-text log path).
#[tokio::test]
async fn test_send_simplified_with_timeout_no_plugin_fallback() {
    let gw = test_gw();
    // No plugin registered — should fallback to plain-text log, not panic.
    let result = send_simplified_with_timeout(&gw, "chat3", "nonexistent", "msg").await;
    assert!(
        result.is_ok(),
        "no-plugin fallback should return Ok, got {:?}",
        result
    );
}

// ===========================================================================
// 1-second response constraint monitoring tests (Steps 1.4-1.5)
// ===========================================================================

/// Shared sentinel check behind the constraint mock's `parse_inbound()`
/// (payload marker `sentinel`). Lossy UTF-8 decoding keeps the substring
/// match working even if the payload carries non-UTF-8 bytes.
fn is_sentinel_payload(payload: &[u8]) -> bool {
    String::from_utf8_lossy(payload).contains("sentinel")
}

/// Parameterized mock plugin shared by the 1-second response constraint
/// tests — one implementation covers the slow and the fast path instead of
/// two near-identical structs. The observable differences live in the
/// fields:
///
/// - `done`: notified when a sentinel payload is parsed. The inbound queue
///   is FIFO, so the sentinel's parse only runs after the request under
///   test has been fully processed — including its 1s-constraint check —
///   giving the tests a deterministic completion signal without fixed
///   sleeps.
/// - `parse_delay`: optional sleep applied in `parse_inbound()` before
///   returning the message. `Some(1500ms)` drives total processing past
///   the 1-second constraint; `None` keeps it far below. The sentinel
///   check precedes the delay so the sentinel itself neither delays nor
///   re-triggers the constraint warn.
/// - `content`: message content returned by `parse_inbound()`, identifying
///   which test a captured message came from.
struct ConstraintMockPlugin {
    done: Arc<tokio::sync::Notify>,
    parse_delay: Option<std::time::Duration>,
    content: &'static str,
}

#[async_trait]
impl IMPlugin for ConstraintMockPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        if is_sentinel_payload(payload) {
            self.done.notify_one();
            return Ok(None);
        }
        // Simulate slow inbound parsing when the test needs to exceed 1s.
        if let Some(delay) = self.parse_delay {
            tokio::time::sleep(delay).await;
        }
        Ok(Some(NormalizedMessage {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: self.content.into(),
            timestamp: chrono::Utc::now().timestamp(),
            message_type: closeclaw_common::MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            reply_ref: None,
            account_id: "u1".into(),
            ..Default::default()
        }))
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        render_plain(content_blocks)
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// When inbound processing exceeds 1 second, the 1-second response constraint
/// monitor should fire a warn log. This test enqueues a message through the
/// consumer with a slow-parse plugin (1500ms parse > 1s constraint) and
/// asserts the captured buffer contains the warn message anchor and the
/// request's `trace_id` field. A sentinel request queued behind the slow
/// one parses only after the slow request's constraint check (queue is
/// FIFO), so the test needs no fixed sleep.
///
/// `#[serial]` per the module log-capture note (issue #3102 callsite race).
#[serial_test::serial]
#[tokio::test(flavor = "current_thread")]
async fn test_1s_response_constraint_warn_log() {
    // The explicit current-thread flavor keeps the spawned inbound consumer
    // on the thread that installed the capture subscriber, so its warn
    // output lands in the thread-local buffer (see `capture_logs_async`'s
    // threading contract).
    let (_result, logs) = capture_logs_async(
        || async {
            let gw = make_gateway_for_1s_test();
            let done = Arc::new(tokio::sync::Notify::new());
            let plugin = ConstraintMockPlugin {
                done: Arc::clone(&done),
                parse_delay: Some(std::time::Duration::from_millis(1500)),
                content: "slow-parse",
            };
            gw.register_plugin(Arc::new(plugin) as Arc<dyn IMPlugin>)
                .await;
            let handle = gw.start_inbound_queue();
            handle.try_send(queued(make_slow_request())).unwrap();
            handle.try_send(queued(make_sentinel_request())).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), done.notified())
                .await
                .expect("slow-path request must finish under the 5s ceiling");
        },
        tracing::Level::WARN,
    )
    .await;
    assert!(
        logs.contains("exceeded 1s response constraint"),
        "1s-constraint warn missing; captured: {logs}"
    );
    assert!(
        logs.contains("trace_id=slow-parse-trace"),
        "trace_id field anchor missing; captured: {logs}"
    );
}

/// Negative dual of [`test_1s_response_constraint_warn_log`]: inbound
/// processing that stays under the 1-second constraint (fast parse) must
/// not fire the monitor — the captured buffer has to distinguish "warn
/// emitted" from "warn not emitted". The sentinel request queued behind
/// the fast one parses only after the fast request's constraint check
/// (queue is FIFO), so the test needs no fixed sleep.
///
/// `#[serial]` per the module log-capture note (issue #3102 callsite race).
#[serial_test::serial]
#[tokio::test(flavor = "current_thread")]
async fn test_no_1s_response_constraint_warn_on_fast_path() {
    // Same capture driver as the positive case: the explicit current-thread
    // flavor keeps the spawned consumer on the subscriber's thread (see
    // `capture_logs_async`'s threading contract).
    let (_result, logs) = capture_logs_async(
        || async {
            let gw = make_gateway_for_1s_test();
            let done = Arc::new(tokio::sync::Notify::new());
            let plugin = ConstraintMockPlugin {
                done: Arc::clone(&done),
                parse_delay: None,
                content: "fast-parse",
            };
            gw.register_plugin(Arc::new(plugin) as Arc<dyn IMPlugin>)
                .await;
            let handle = gw.start_inbound_queue();
            handle.try_send(queued(make_fast_request())).unwrap();
            handle.try_send(queued(make_sentinel_request())).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), done.notified())
                .await
                .expect("fast-path request must finish under the 5s ceiling");
        },
        tracing::Level::WARN,
    )
    .await;
    assert!(
        !logs.contains("exceeded 1s response constraint"),
        "sub-second processing must not emit the constraint warn; captured: {logs}"
    );
    assert!(
        !logs.contains("trace_id=fast-parse-trace"),
        "constraint warn trace_id must be absent on the fast path; captured: {logs}"
    );
}

fn make_gateway_for_1s_test() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "outbound_helpers_1s_test".into(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 4,
        inbound_wal_dir: None,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(&config, None, None, Default::default()));
    Arc::new(Gateway::new_for_tests(config, sm))
}

fn make_slow_request() -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        raw_payload: b"{}".to_vec(),
        peer_id: "p1".into(),
        trace_id: "slow-parse-trace".into(),
        span_id: None,
    }
}

fn make_fast_request() -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        raw_payload: br#"{"kind":"fast"}"#.to_vec(),
        peer_id: "p1".into(),
        trace_id: "fast-parse-trace".into(),
        span_id: None,
    }
}

/// Sentinel queued behind the request under test (slow or fast): its parse
/// marks that the request's response-constraint check has already run
/// (queue is FIFO).
fn make_sentinel_request() -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        raw_payload: br#"{"kind":"sentinel"}"#.to_vec(),
        peer_id: "p1".into(),
        trace_id: "sentinel-trace".into(),
        span_id: None,
    }
}

// ---------------------------------------------------------------------------
// is_sentinel_payload unit tests
// ---------------------------------------------------------------------------

/// Positive: the real sentinel fixture payload (same bytes the constraint
/// tests queue via `make_sentinel_request`) matches.
#[test]
fn test_is_sentinel_payload_matches_real_fixture() {
    assert!(
        is_sentinel_payload(make_sentinel_request().raw_payload.as_slice()),
        "expected the real sentinel fixture payload to match; actual payload: {:?}",
        String::from_utf8_lossy(make_sentinel_request().raw_payload.as_slice())
    );
}

/// Negative normal-path payloads: `{}` (slow fixture) and `{"kind":"fast"}`
/// (fast fixture) must not trip the sentinel check.
#[test]
fn test_is_sentinel_payload_rejects_non_sentinel_payloads() {
    assert!(
        !is_sentinel_payload(make_slow_request().raw_payload.as_slice()),
        "expected no match for the slow fixture payload; actual payload: {:?}",
        String::from_utf8_lossy(make_slow_request().raw_payload.as_slice())
    );
    assert!(
        !is_sentinel_payload(make_fast_request().raw_payload.as_slice()),
        "expected no match for the fast fixture payload; actual payload: {:?}",
        String::from_utf8_lossy(make_fast_request().raw_payload.as_slice())
    );
}

/// Boundary: an empty payload carries no marker at all.
#[test]
fn test_is_sentinel_payload_empty_payload() {
    assert!(
        !is_sentinel_payload(&[]),
        "expected no match for an empty payload; actual payload: {:?}",
        String::from_utf8_lossy(&[])
    );
}

/// Boundary: non-UTF-8 bytes around the marker still match — lossy decoding
/// replaces them with U+FFFD without breaking the substring hit.
#[test]
fn test_is_sentinel_payload_matches_despite_non_utf8_bytes() {
    assert!(
        is_sentinel_payload(b"\xffsentinel\xff"),
        "expected a match despite non-UTF-8 bytes around the marker; actual payload: {:?}",
        String::from_utf8_lossy(b"\xffsentinel\xff")
    );
}

// ---------------------------------------------------------------------------
// Step 1.15: deliver_batch_result branch coverage
// ---------------------------------------------------------------------------

/// Fixture: one created session (record + ConversationSession) and a
/// Gateway holding the SessionManager used for setup assertions —
/// `deliver_batch_result` sources the SessionManager from the Gateway
/// itself (Step 1.20).
///
/// `with_chain` injects the default processor chain — required by the case
/// that actually reaches `deliver_batch_result` → `send_outbound` →
/// `process_or_bypass`; the short-circuit cases (send must never run) keep
/// the chain-less construction.
async fn deliver_batch_fixture(with_chain: bool) -> (Arc<Gateway>, Arc<SessionManager>, String) {
    let config = GatewayConfig {
        name: "outbound_helpers_deliver_batch".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(&config, None, None, Default::default()));
    let sid = sm
        .find_or_create("mock", &make_msg(), None)
        .await
        .expect("session creation must succeed");
    let gw = if with_chain {
        let chain = crate::processor_registry_test_utils::default_registry(&config);
        Arc::new(Gateway::with_processor_registry(
            config,
            Arc::clone(&sm),
            chain,
        ))
    } else {
        Arc::new(Gateway::new_for_tests(config, Arc::clone(&sm)))
    };
    (gw, sm, sid)
}

/// Streaming-skip branch: streaming turns were already delivered chunk by
/// chunk via `send_outbound_streaming`, so `deliver_batch_result` must
/// return before the channel lookup / `send_outbound` (the same fixture
/// proven to deliver in the non-streaming test below).
#[tokio::test]
async fn test_deliver_batch_result_skips_streaming_turn() {
    let (plugin, tracker) = make_plugin();
    let (gw, sm, sid) = deliver_batch_fixture(false).await;
    gw.register_plugin(plugin).await;
    // Session record exists, so the skip can only come from the streaming
    // check — not from a missing channel.
    assert!(
        sm.has_session(&sid).await,
        "fixture must have a session record"
    );
    let cs = sm
        .get_conversation_session(&sid)
        .await
        .expect("conversation session");
    cs.write().await.set_stream_enabled(true);
    assert!(
        cs.read().await.stream_enabled(),
        "fixture must be streaming"
    );

    deliver_batch_result(&gw, &sid, "hello", &[ContentBlock::Text("hello".into())]).await;

    assert!(
        !tracker.was_send_called(),
        "streaming turn must be skipped: send_outbound must not run"
    );
}

/// Missing-channel branch: the session record that carries the channel
/// is gone while the conversation session still resolves → warn + return
/// before `send_outbound`; no panic, the plugin is never asked to send.
#[tokio::test]
async fn test_deliver_batch_result_missing_session_record_returns_without_send() {
    let (plugin, tracker) = make_plugin();
    let (gw, sm, sid) = deliver_batch_fixture(false).await;
    gw.register_plugin(plugin).await;
    // Drop the record that carries `channel`; the conversation session
    // stays so the streaming check still resolves (as non-streaming).
    sm.sessions.write().await.remove(&sid);
    assert!(!sm.has_session(&sid).await, "record must be gone");
    assert!(
        sm.get_conversation_session(&sid).await.is_some(),
        "conversation session must remain"
    );

    deliver_batch_result(&gw, &sid, "hello", &[ContentBlock::Text("hello".into())]).await;

    assert!(
        !tracker.was_send_called(),
        "no channel on session record → warn + return; plugin must not send"
    );
}

/// Renders an unknown `msg_type`, which makes `send_outbound` fail AFTER
/// `plugin.send` succeeded (`extract_content_for_checkpoint` rejects the
/// type) — exercising `deliver_batch_result`'s `send_outbound` failure arm.
struct UnknownMsgTypePlugin {
    sends: std::sync::atomic::AtomicU32,
}

#[async_trait]
impl IMPlugin for UnknownMsgTypePlugin {
    fn platform(&self) -> &str {
        "mock"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "video".into(),
            payload: serde_json::json!({}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.sends.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

/// `send_outbound` failure branch: the error is logged and swallowed —
/// `deliver_batch_result` returns normally (the turn is already in the
/// conversation history) while the send itself was attempted exactly once.
#[tokio::test]
async fn test_deliver_batch_result_swallows_send_outbound_failure() {
    let plugin = Arc::new(UnknownMsgTypePlugin {
        sends: std::sync::atomic::AtomicU32::new(0),
    });
    let (gw, _sm, sid) = deliver_batch_fixture(true).await;
    gw.register_plugin(plugin.clone()).await;

    deliver_batch_result(&gw, &sid, "hello", &[ContentBlock::Text("hello".into())]).await;

    assert_eq!(
        plugin.sends.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "send_outbound must run once and its failure must be swallowed without panic"
    );
}
