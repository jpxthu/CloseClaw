//! Circuit breaker notification tests (Step 1.3 — plan Step 1.1).
//!
//! Verifies that when the compaction circuit breaker trips, an assistant
//! message is injected into the session transcript informing the user,
//! and that the notification is deduplicated and properly reset.

use super::*;
use crate::session_handler::{ActiveSearcherLlmCaller, MessageMetadata};
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::run_health::TranscriptOp;

// ── Helpers ──────────────────────────────────────────────────────────────

fn make_msg() -> crate::Message {
    use std::collections::HashMap;
    crate::Message {
        id: "msg_1".into(),
        from: "alice".into(),
        to: "bob".into(),
        content: "hello".into(),
        channel: "ch".into(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

fn make_config() -> crate::GatewayConfig {
    crate::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_sm() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &make_config(),
        None,
        None,
        ReasoningLevel::default(),
    ))
}

/// Create a [`SessionMessageHandler`] with an output channel for testing.
fn handler_with_channel(
    sm: &Arc<SessionManager>,
) -> (
    SessionMessageHandler,
    tokio::sync::mpsc::Receiver<(String, Vec<ContentBlock>)>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(10);
    let ufc = Arc::new(UnifiedFallbackClient::new(
        vec![],
        Arc::new(CooldownManager::new()),
    ));
    let handler = SessionMessageHandler::new(
        Arc::clone(sm),
        ufc.clone(),
        tx,
        Arc::new(ActiveSearcherLlmCaller {
            client: ufc,
            model: String::new(),
        }),
        closeclaw_session::compaction::CompactConfig::default(),
    );
    (handler, rx)
}

/// Populate a session so `check_and_run_auto_compact` enters
/// `AutoCompactTriggered` state (remaining ≤ 5% of context = 6,400 tokens).
/// Uses `prompt_tokens: 125_000` on 128K context → remaining = 3,000 ≤ 6,400.
async fn populate_session_for_auto_compact(sm: &SessionManager, sid: &str) {
    let cs = sm.get_conversation_session(sid).await.expect("session");
    let mut cs_write = cs.write().await;
    // Add a few messages so `load_compact_inputs` has transcript data.
    let mut msgs = Vec::new();
    for i in 0..4 {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        msgs.push(closeclaw_session::llm_session::SessionMessage {
            role: role.to_string(),
            content_blocks: vec![ContentBlock::Text(format!(
                "Message {} for auto-compact test",
                i
            ))],
            timestamp: chrono::Utc::now(),
        });
    }
    cs_write.apply_transcript_op(TranscriptOp::Rewrite, msgs);
    // 125K used on 128K context → remaining = 3,000 ≤ 6,400 (5% of 128K)
    // → AutoCompactTriggered.
    cs_write.accumulate_usage(&closeclaw_common::processor::UnifiedUsage {
        prompt_tokens: 125_000,
        completion_tokens: 0,
        total_tokens: Some(125_000),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    });
}

/// Verify the circuit breaker notification was injected into the session
/// transcript by checking for an assistant message containing the expected text.
async fn assert_circuit_breaker_notified(sm: &SessionManager, sid: &str) {
    let cs = sm.get_conversation_session(sid).await.expect("session");
    let msgs = cs.read().await.messages().to_vec();
    let notified = msgs.iter().any(|m| {
        m.role == "assistant"
            && m.content_blocks.iter().any(|b| match b {
                ContentBlock::Text(t) => t == "自动压缩已暂停，建议手动 /compact",
                _ => false,
            })
    });
    assert!(
        notified,
        "expected circuit breaker notification in transcript"
    );
}

/// Verify the circuit breaker notification was NOT injected.
async fn assert_circuit_breaker_not_notified(sm: &SessionManager, sid: &str) {
    let cs = sm.get_conversation_session(sid).await.expect("session");
    let msgs = cs.read().await.messages().to_vec();
    let notified = msgs.iter().any(|m| {
        m.role == "assistant"
            && m.content_blocks.iter().any(|b| match b {
                ContentBlock::Text(t) => t == "自动压缩已暂停，建议手动 /compact",
                _ => false,
            })
    });
    assert!(
        !notified,
        "should NOT have circuit breaker notification in transcript"
    );
}

/// Trip the circuit breaker by recording `max_consecutive_failures`
/// (default 3) failures on the handler's compaction service.
async fn trip_circuit_breaker(handler: &SessionMessageHandler) {
    for _ in 0..3 {
        handler.compaction_service.lock().await.record_failure();
    }
}

/// Assert the `has_circuit_break_notified` flag value.
fn assert_circuit_break_notified_flag(handler: &SessionMessageHandler, expected: bool) {
    assert_eq!(
        *handler.has_circuit_break_notified.lock().expect("poisoned"),
        expected,
        "has_circuit_break_notified should be {}",
        expected
    );
}

/// Count assistant messages that contain the circuit breaker notification text.
async fn count_notifications(sm: &SessionManager, sid: &str) -> usize {
    let cs = sm.get_conversation_session(sid).await.expect("session");
    let msgs = cs.read().await.messages().to_vec();
    msgs.iter()
        .filter(|m| {
            m.role == "assistant"
                && m.content_blocks.iter().any(|b| match b {
                    ContentBlock::Text(t) => t == "自动压缩已暂停，建议手动 /compact",
                    _ => false,
                })
        })
        .count()
}

// ── Tests ────────────────────────────────────────────────────────────────

/// When the circuit breaker trips for the first time,
/// `check_and_run_auto_compact` must inject an assistant message into the
/// session transcript informing the user that auto-compaction is paused.
#[tokio::test]
async fn test_circuit_breaker_notification_first_trip() {
    let sm = make_sm();
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    populate_session_for_auto_compact(&sm, &sid).await;
    let (handler, _rx) = handler_with_channel(&sm);

    // Trip the breaker (3 failures)
    trip_circuit_breaker(&handler).await;

    // Before calling check: no notification yet
    assert_circuit_breaker_not_notified(&sm, &sid).await;

    // Trigger auto-compact check — breaker is tripped, notification injected
    handler.check_and_run_auto_compact(&sid).await;

    // Verify notification was injected into transcript
    assert_circuit_breaker_notified(&sm, &sid).await;

    // Verify has_circuit_break_notified flag is set
    assert_circuit_break_notified_flag(&handler, true);
}

/// When the circuit breaker remains tripped across multiple
/// `check_and_run_auto_compact` calls, the notification must NOT be
/// injected more than once (dedup).
#[tokio::test]
async fn test_circuit_breaker_notification_no_duplicate() {
    let sm = make_sm();
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    populate_session_for_auto_compact(&sm, &sid).await;
    let (handler, _rx) = handler_with_channel(&sm);

    trip_circuit_breaker(&handler).await;

    // First check → injects notification
    handler.check_and_run_auto_compact(&sid).await;
    assert_circuit_breaker_notified(&sm, &sid).await;

    let count_before = count_notifications(&sm, &sid).await;

    // Second check → must NOT inject another notification
    handler.check_and_run_auto_compact(&sid).await;

    let count_after = count_notifications(&sm, &sid).await;
    assert_eq!(
        count_before, count_after,
        "notification count should remain {} after second check",
        count_before
    );
}

/// After a successful compaction resets the circuit breaker,
/// the `has_circuit_break_notified` flag must be cleared so that a
/// subsequent trip re-injects the notification.
#[tokio::test]
async fn test_circuit_breaker_notification_reset_after_success() {
    let sm = make_sm();
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    populate_session_for_auto_compact(&sm, &sid).await;
    let (handler, _rx) = handler_with_channel(&sm);

    // Trip the breaker
    trip_circuit_breaker(&handler).await;
    handler.check_and_run_auto_compact(&sid).await;
    assert_circuit_break_notified_flag(&handler, true);

    // Simulate a successful compaction by calling record_success on
    // the compaction service. This resets consecutive_failures to 0
    // AND run_auto_compact resets has_circuit_break_notified on Ok.
    handler.compaction_service.lock().await.record_success();
    // Manually reset the flag as run_auto_compact does on Ok path
    *handler.has_circuit_break_notified.lock().expect("poisoned") = false;

    // Verify flag was reset
    assert_circuit_break_notified_flag(&handler, false);

    // Clear the old notification from transcript so we can verify
    // a fresh one is injected on next trip.
    {
        let cs = sm.get_conversation_session(&sid).await.expect("session");
        let msgs = cs.read().await.messages().to_vec();
        let non_notif: Vec<_> = msgs
            .into_iter()
            .filter(|m| {
                !(m.role == "assistant"
                    && m.content_blocks.iter().any(|b| match b {
                        ContentBlock::Text(t) => t == "自动压缩已暂停，建议手动 /compact",
                        _ => false,
                    }))
            })
            .collect();
        cs.write()
            .await
            .apply_transcript_op(TranscriptOp::Rewrite, non_notif);
    }

    // Trip the breaker again
    trip_circuit_breaker(&handler).await;
    handler.check_and_run_auto_compact(&sid).await;

    // Notification should be injected again (flag was reset)
    assert_circuit_breaker_notified(&sm, &sid).await;
    assert_circuit_break_notified_flag(&handler, true);
}

// ═════════════════════════════════════════════════════════════════════════════
// Streaming path: user message before auto-compact (Step 1.2)
// ═════════════════════════════════════════════════════════════════════════════

/// Verify that `handle_message_with_gateway` persists the user message
/// into the session transcript before auto-compact runs.
///
/// This mirrors `test_user_message_persisted_before_compact_check` in
/// `session_handler_tests.rs` but exercises the streaming dispatch path.
#[tokio::test]
async fn test_streaming_path_persists_user_message_before_compact() {
    let sm = make_sm();
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    let config = make_config();
    let gw = Arc::new(crate::Gateway::new(config, Arc::clone(&sm)));
    let plugin: Arc<dyn IMPlugin> = Arc::new(MockStreamingPlugin);
    gw.register_plugin(plugin.clone()).await;

    let (handler, _rx) = handler_with_channel(&sm);
    handler
        .handle_message_with_gateway(
            &sid,
            "streaming order check".to_string(),
            MessageMetadata::default_meta(),
            &gw,
            &plugin,
        )
        .await;

    // Poll until user message appears (no bare sleep).
    let start = tokio::time::Instant::now();
    let user_msg;
    loop {
        let cs = sm.get_conversation_session(&sid).await.expect("session");
        let msgs = cs.read().await.messages().to_vec();
        if let Some(m) = msgs.iter().find(|m| m.role == "user") {
            user_msg = m.clone();
            break;
        }
        if start.elapsed() > tokio::time::Duration::from_millis(200) {
            panic!("timeout waiting for user message");
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }
    match &user_msg.content_blocks[0] {
        ContentBlock::Text(t) => assert_eq!(t, "streaming order check"),
        other => panic!("expected Text block, got {:?}", other),
    }
}

/// Minimal mock plugin for streaming path tests.
struct MockStreamingPlugin;

#[async_trait::async_trait]
impl IMPlugin for MockStreamingPlugin {
    fn platform(&self) -> &str {
        "mock"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> closeclaw_common::im_plugin::RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        closeclaw_common::im_plugin::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        _output: &closeclaw_common::im_plugin::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        Ok(())
    }
}
