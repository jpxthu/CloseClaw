//! Unit tests for `llm_caller` module.
//!
//! Verifies that `InternalMessage` construction patterns work correctly
//! with the `tool_call_id` field (added in PR #1299) and that the
//! `call_llm` / `call_llm_streaming` request construction uses `None`.

use closeclaw_llm::session::{InjectionPosition, MemoryInjection};
use closeclaw_llm::types::InternalMessage;
use std::path::PathBuf;
use std::sync::Arc;

// ── InternalMessage construction patterns ────────────────────────────────────

#[test]
fn test_internal_message_default_has_no_tool_call_id() {
    let msg = InternalMessage::default();
    assert_eq!(msg.role, "");
    assert_eq!(msg.content, "");
    assert!(msg.tool_call_id.is_none());
}

#[test]
fn test_internal_message_with_spread_default() {
    // Simulates the pattern: InternalMessage { role, content, ..Default::default() }
    let msg = InternalMessage {
        role: "user".to_string(),
        content: "hello".to_string(),
        ..Default::default()
    };
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content, "hello");
    assert!(msg.tool_call_id.is_none());
}

#[test]
fn test_internal_message_with_explicit_none() {
    // Simulates the pattern: InternalMessage { role, content, tool_call_id: None }
    let msg = InternalMessage {
        role: "assistant".to_string(),
        content: "response".to_string(),
        tool_call_id: None,
    };
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.content, "response");
    assert!(msg.tool_call_id.is_none());
}

#[test]
fn test_internal_message_preserves_tool_call_id_when_set() {
    // Ensure the field still works when explicitly provided
    let msg = InternalMessage {
        role: "tool".to_string(),
        content: "result".to_string(),
        tool_call_id: Some("call_abc123".to_string()),
    };
    assert_eq!(msg.tool_call_id.as_deref(), Some("call_abc123"));
}

// ── Serialization round-trip ─────────────────────────────────────────────────

#[test]
fn test_internal_message_serializes_without_tool_call_id() {
    let msg = InternalMessage {
        role: "user".to_string(),
        content: "hi".to_string(),
        tool_call_id: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    // tool_call_id should be skipped when None (serde skip_serializing_if)
    assert!(!json.contains("tool_call_id"));
    assert!(json.contains("\"role\""));
    assert!(json.contains("\"content\""));
}

#[test]
fn test_internal_message_serializes_with_tool_call_id() {
    let msg = InternalMessage {
        role: "tool".to_string(),
        content: "result".to_string(),
        tool_call_id: Some("call_xyz".to_string()),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"tool_call_id\""));
    assert!(json.contains("call_xyz"));
}

#[test]
fn test_internal_message_deserializes_without_tool_call_id() {
    let json = r#"{"role":"user","content":"test"}"#;
    let msg: InternalMessage = serde_json::from_str(json).unwrap();
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content, "test");
    assert!(msg.tool_call_id.is_none());
}

// ── memory_injection_to_message ──────────────────────────────────────────────

use crate::llm_caller::memory_injection_to_message;

#[test]
fn test_memory_injection_to_message_has_tool_role() {
    let inj = MemoryInjection::new("summary text".to_string(), InjectionPosition::AfterCurrent);
    let msg = memory_injection_to_message(&inj);
    assert_eq!(msg.role, "tool");
    assert_eq!(msg.content, "summary text");
    assert!(msg.tool_call_id.is_none());
}

#[test]
fn test_memory_injection_to_message_preserves_content() {
    let inj = MemoryInjection::new(
        "detailed context about the conversation".to_string(),
        InjectionPosition::BeforeNext,
    );
    let msg = memory_injection_to_message(&inj);
    assert_eq!(msg.content, "detailed context about the conversation");
}

// ── consume_memory_injection ─────────────────────────────────────────────────

use crate::llm_caller::consume_memory_injection;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;

fn make_injection(content: &str, pos: InjectionPosition) -> MemoryInjection {
    MemoryInjection::new(content.to_string(), pos)
}

async fn setup_session_with_injection(
    content: &str,
    pos: InjectionPosition,
) -> (Arc<crate::session_manager::SessionManager>, String) {
    use crate::session_manager::SessionManager;
    use crate::{GatewayConfig, Session};
    use chrono::Utc;
    use closeclaw_llm::session::ConversationSession;
    use std::sync::Arc;

    let config = GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    };
    let mgr = SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    {
        mgr.sessions.write().await.insert(
            session_id.clone(),
            Session {
                id: session_id.clone(),
                agent_id: "test-agent".to_string(),
                channel: "feishu".to_string(),
                created_at: Utc::now().timestamp(),
                depth: 0,
            },
        );
        let cs = Arc::new(tokio::sync::RwLock::new(ConversationSession::new(
            session_id.clone(),
            "test-model".to_string(),
            PathBuf::from("/tmp"),
        )));
        {
            let cs_ref = cs.read().await;
            cs_ref.set_memory_injection(make_injection(content, pos));
        }
        mgr.conversation_sessions
            .write()
            .await
            .insert(session_id.clone(), cs);
    }
    (Arc::new(mgr), session_id)
}

#[tokio::test]
async fn test_consume_injection_after_current() {
    let (mgr, sid) =
        setup_session_with_injection("context after user", InjectionPosition::AfterCurrent).await;
    let inj = consume_memory_injection(&mgr, &sid).await;
    let inj = inj.expect("should have injection");
    assert_eq!(inj.content, "context after user");
    assert_eq!(inj.position_mode, InjectionPosition::AfterCurrent);
}

#[tokio::test]
async fn test_consume_injection_before_next() {
    let (mgr, sid) =
        setup_session_with_injection("context before next", InjectionPosition::BeforeNext).await;
    let inj = consume_memory_injection(&mgr, &sid).await;
    let inj = inj.expect("should have injection");
    assert_eq!(inj.content, "context before next");
    assert_eq!(inj.position_mode, InjectionPosition::BeforeNext);
}

#[tokio::test]
async fn test_consume_empty_slot_returns_none() {
    let (mgr, sid) =
        setup_session_with_injection("should be consumed", InjectionPosition::AfterCurrent).await;
    // First consume: should return Some
    let first = consume_memory_injection(&mgr, &sid).await;
    assert!(first.is_some(), "first consume should return Some");
    // Second consume: slot is cleared, should return None
    let second = consume_memory_injection(&mgr, &sid).await;
    assert!(second.is_none(), "second consume should return None");
}

#[tokio::test]
async fn test_consume_nonexistent_session_returns_none() {
    use crate::session_manager::SessionManager;
    use crate::GatewayConfig;

    let config = GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    };
    let mgr = SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let result = consume_memory_injection(&Arc::new(mgr), "nonexistent-id").await;
    assert!(result.is_none());
}
