//! Step 1.3: Unit tests for agent_id explicit passing and queuing notification.
//!
//! Test dimensions:
//! 1. agent_id parameter controls per-agent lock serialization
//! 2. agent_id different from message.to → lock uses agent_id
//! 3. Empty agent_id edge case
//! 4. Queuing notification text comes from Session's QUEUING_NOTIFICATION_TEXT

use super::tests::{make_test_mgr, test_config};
use super::SessionManager;
use crate::session_handler::{HandleResult, MessageMetadata, SessionMessageHandler};
use crate::Message;
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::LLMRegistry;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

fn test_message() -> Message {
    Message {
        id: "msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

fn build_handler(sm: Arc<SessionManager>) -> SessionMessageHandler {
    let registry = Arc::new(LLMRegistry::new());
    let fallback = Arc::new(FallbackClient::from_strings(registry, vec![]));
    let ufc = Arc::new(UnifiedFallbackClient::new(
        vec![],
        Arc::new(CooldownManager::new()),
    ));
    let fallback_llm_caller = Arc::new(crate::session_handler::ActiveSearcherLlmCaller {
        client: ufc,
        model: String::new(),
    });
    SessionMessageHandler::new_no_output(
        sm,
        fallback,
        fallback_llm_caller,
        closeclaw_session::compaction::CompactConfig::default(),
    )
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. agent_id parameter controls per-agent lock serialization
// ═════════════════════════════════════════════════════════════════════════════

/// When `agent_id` parameter differs from `message.to`, the per-agent lock
/// entry should be keyed by `agent_id` (the parameter), not `message.to`.
///
/// This verifies the design doc requirement: "Gateway 将 agent_id 连同
/// session_key、路由字段一并传给 SessionManager" — the agent_id parameter
/// is the source of truth for lock serialization.
#[tokio::test]
async fn test_resolve_uses_agent_id_param_for_lock_not_message_to() {
    let mgr = make_test_mgr(None);
    let msg = test_message(); // to = "agent-b"

    // Call resolve with agent_id = "agent-c" (different from message.to)
    let result = mgr
        .resolve("sk-1", "feishu", &msg, None, "agent-c")
        .await
        .unwrap();

    // Session should be created successfully
    assert!(!result.is_empty());

    // agent_locks should have entry for "agent-c" (the parameter), NOT "agent-b"
    let locks = mgr.agent_locks.read().await;
    assert!(
        locks.contains_key("agent-c"),
        "lock should be keyed by agent_id param (agent-c), got keys: {:?}",
        locks.keys().collect::<Vec<_>>()
    );
    assert!(
        !locks.contains_key("agent-b"),
        "lock should NOT be keyed by message.to (agent-b)"
    );
}

/// Verify that two concurrent resolves for the same `agent_id` parameter
/// (even with different `message.to`) share the same lock.
#[tokio::test]
async fn test_same_agent_id_param_shares_lock() {
    let mgr = Arc::new(make_test_mgr(None));

    let msg1 = Message {
        id: "msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-x".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let msg2 = Message {
        id: "msg-2".to_string(),
        from: "user-b".to_string(),
        to: "agent-y".to_string(),
        content: "world".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };

    // Both use agent_id = "shared-agent" but different message.to
    let _r1 = mgr
        .resolve("sk-1", "feishu", &msg1, None, "shared-agent")
        .await;
    let _r2 = mgr
        .resolve("sk-2", "feishu", &msg2, None, "shared-agent")
        .await;

    let locks = mgr.agent_locks.read().await;
    assert_eq!(
        locks.len(),
        1,
        "should have exactly 1 lock for shared-agent"
    );
    assert!(locks.contains_key("shared-agent"));
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Empty agent_id edge case
// ═════════════════════════════════════════════════════════════════════════════

/// When `agent_id` is an empty string, resolve should still create a session
/// successfully. The agent_locks map should have an entry for "".
#[tokio::test]
async fn test_resolve_empty_agent_id_creates_session() {
    let mgr = make_test_mgr(None);
    let msg = test_message(); // to = "agent-b"

    let result = mgr.resolve("sk-empty", "feishu", &msg, None, "").await;
    assert!(result.is_ok(), "resolve with empty agent_id should succeed");
    let session_id = result.unwrap();
    assert!(!session_id.is_empty());

    // agent_locks should have entry for "" (empty string)
    let locks = mgr.agent_locks.read().await;
    assert!(
        locks.contains_key(""),
        "lock should be keyed by empty agent_id"
    );
}

/// When `agent_id` is empty and `message.to` is non-empty, the session should
/// still be created with the correct session_id prefix (from message.to).
#[tokio::test]
async fn test_resolve_empty_agent_id_session_uses_message_to_for_id() {
    let mgr = make_test_mgr(None);
    let msg = test_message(); // to = "agent-b"

    let session_id = mgr.resolve("sk-2", "feishu", &msg, None, "").await.unwrap();
    // session_id is generated from message.to (agent_b), not agent_id ("")
    assert!(
        session_id.starts_with("agent-b_"),
        "session_id should use message.to for prefix, got: {}",
        session_id
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Queuing notification text from Session
// ═════════════════════════════════════════════════════════════════════════════

/// When a session is busy and a message arrives, `handle_message_with_meta`
/// returns `HandleResult::MessageQueued` carrying the notification text from
/// Session's `QUEUING_NOTIFICATION_TEXT` constant.
#[tokio::test]
async fn test_queuing_notification_text_from_session() {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    let handler = build_handler(Arc::clone(&sm));

    // Create a session and set it to busy
    let msg = test_message();
    let session_id = sm.find_or_create("feishu", &msg, None).await.unwrap();
    if let Some(cs) = sm.get_conversation_session(&session_id).await {
        let cs = cs.write().await;
        cs.set_llm_busy(true);
        cs.set_llm_state(LlmState::Requesting);
    }

    // Send a message to the busy session
    let result = handler
        .handle_message_with_meta(
            &session_id,
            "second message".to_string(),
            MessageMetadata::default_meta(),
        )
        .await;

    // Verify MessageQueued carries the correct text
    match result {
        HandleResult::MessageQueued(text) => {
            assert_eq!(
                text,
                crate::session_handler::QUEUING_NOTIFICATION_TEXT,
                "notification text should match Session's QUEUING_NOTIFICATION_TEXT constant"
            );
        }
        other => {
            panic!("expected MessageQueued, got {:?}", other);
        }
    }
}
