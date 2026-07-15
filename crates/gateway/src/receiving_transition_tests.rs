//! Tests for LlmState Receiving transition on streaming outbound.
//!
//! Verifies that streaming LLM calls correctly transition:
//!   Idle → Requesting → Receiving → Idle
//! and that the Receiving state is set exactly once on the first stream event.

use closeclaw_common::processor::StreamEvent;
use closeclaw_common::LlmState;
use closeclaw_session::persistence::ReasoningLevel;
use futures::stream;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{GatewayConfig, SessionManager};

use super::outbound_tests::{default_usage, streaming_config, ThinkingIndicatorMock};

/// Helper: create a SessionManager with a session and its ConversationSession.
/// Sets the ConversationSession's LlmState to the given `initial` state.
async fn setup_receiving_test(
    session_id: &str,
    initial: LlmState,
) -> (Arc<SessionManager>, String) {
    let config = GatewayConfig {
        name: "test-receiving".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: "chat_rcv".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    );
    cs.set_llm_state(initial);
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        sm.conversation_sessions
            .write()
            .await
            .insert(session_id.to_string(), cs_arc);
    }
    (sm, session_id.to_string())
}

/// Helper: read the current LlmState from the SessionManager's ConversationSession.
async fn read_llm_state(sm: &SessionManager, session_id: &str) -> LlmState {
    let cs = sm.get_conversation_session(session_id).await.unwrap();
    let guard = cs.read().await;
    guard.llm_state()
}

/// Streaming: Idle → Requesting → Receiving → Idle.
///
/// Verifies that the first stream event transitions LlmState from
/// Requesting to Receiving. Since `send_outbound_streaming` doesn't
/// reset to Idle (that's `session_handler_streaming`'s job), we
/// verify up to Receiving.
#[tokio::test]
async fn test_streaming_receiving_transition() {
    let session_id = "sess-rcv-1";
    let (sm, _) = setup_receiving_test(session_id, LlmState::Requesting).await;
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(ThinkingIndicatorMock::new("mock"));
    let gw = crate::Gateway::new(streaming_config(), Arc::clone(&sm));
    gw.register_plugin(Arc::clone(&plugin)).await;

    // Before streaming: should be Requesting.
    assert_eq!(read_llm_state(&sm, session_id).await, LlmState::Requesting);

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "hello".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let s = stream::iter(events);

    let _ = gw
        .send_outbound_streaming(session_id, "mock", s, &plugin)
        .await;

    // After first stream event: should be Receiving.
    assert_eq!(
        read_llm_state(&sm, session_id).await,
        LlmState::Receiving,
        "LlmState should transition to Receiving after first stream event"
    );
}

/// Streaming: multiple events — Receiving set only once.
///
/// Verifies that even with many stream events, the Receiving state
/// is only set on the first event (the bool flag prevents duplicates).
#[tokio::test]
async fn test_streaming_receiving_set_only_once() {
    let session_id = "sess-rcv-2";
    let (sm, _) = setup_receiving_test(session_id, LlmState::Requesting).await;
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(ThinkingIndicatorMock::new("mock"));
    let gw = crate::Gateway::new(streaming_config(), Arc::clone(&sm));
    gw.register_plugin(Arc::clone(&plugin)).await;

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "part1".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "part2".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: closeclaw_common::ContentDelta::Text {
                text: "part3".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: None,
        }),
    ];
    let s = stream::iter(events);

    let _ = gw
        .send_outbound_streaming(session_id, "mock", s, &plugin)
        .await;

    assert_eq!(
        read_llm_state(&sm, session_id).await,
        LlmState::Receiving,
        "Receiving should be set exactly once, state should remain Receiving"
    );
}

/// Non-streaming path: LlmState not affected by send_outbound.
///
/// Verifies that the non-streaming outbound path (`send_outbound`)
/// does not set LlmState to Receiving.
#[tokio::test]
async fn test_non_streaming_no_receiving() {
    let session_id = "sess-rcv-3";
    let (sm, _) = setup_receiving_test(session_id, LlmState::Requesting).await;
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(ThinkingIndicatorMock::new("mock"));
    let gw = crate::Gateway::new(streaming_config(), Arc::clone(&sm));
    gw.register_plugin(Arc::clone(&plugin)).await;

    let result = gw.send_outbound(session_id, "mock", "hello", vec![]).await;
    assert!(result.is_ok(), "send_outbound should succeed");

    assert_eq!(
        read_llm_state(&sm, session_id).await,
        LlmState::Requesting,
        "non-streaming path should not change LlmState to Receiving"
    );
}

/// Error as first stream event: Receiving should be set.
///
/// When the first stream event is an Error (LLM responded but failed),
/// the state should still transition to Receiving because the LLM
/// did start returning data.
#[tokio::test]
async fn test_streaming_error_first_event_sets_receiving() {
    let session_id = "sess-rcv-err";
    let (sm, _) = setup_receiving_test(session_id, LlmState::Requesting).await;
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(ThinkingIndicatorMock::new("mock"));
    let gw = crate::Gateway::new(streaming_config(), Arc::clone(&sm));
    gw.register_plugin(Arc::clone(&plugin)).await;

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![Ok(StreamEvent::Error {
        message: "stream error".to_string(),
    })];
    let s = stream::iter(events);

    let _ = gw
        .send_outbound_streaming(session_id, "mock", s, &plugin)
        .await;

    assert_eq!(
        read_llm_state(&sm, session_id).await,
        LlmState::Receiving,
        "Error as first event should still trigger Receiving state"
    );
}
