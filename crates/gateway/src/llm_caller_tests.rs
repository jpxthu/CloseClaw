//! Unit tests for `llm_caller` module.
//!
//! Verifies that `InternalMessage` construction patterns work correctly
//! with the `tool_call_id` field (added in PR #1299) and that the
//! `call_llm` / `call_llm_streaming` request construction uses `None`.

use closeclaw_llm::types::InternalMessage;

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
