//! Step 1.7: system_appends split — persistence backward-compat tests.
//!
//! Tests that the checkpoint JSON serialization/deserialization handles
//! the old `system_appends` alias and new `user_appends` field correctly,
//! and that `system_injection_appends` is never persisted.

use crate::persistence::SessionCheckpoint;

/// Old format JSON with "system_appends" key (no "user_appends")
/// must deserialize into `user_appends` via serde alias.
#[test]
fn test_persistence_old_system_appends_key_alias() {
    let old_json = r#"{
        "session_id": "alias-test",
        "system_appends": ["cmd1", "cmd2"],
        "reasoning_mode": "direct",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "ttl_seconds": 604800,
        "status": "active",
        "message_count": 0,
        "reasoning_level": "high",
        "outbound_pending": [],
        "mode_state": {"current_step": 0, "total_steps": 0, "step_messages": [], "is_complete": false},
        "pending_operations": [],
        "pending_tool_failures": [],
        "progress_tool_calls": [],
        "approval_tool_calls": [],
        "plan_references": [],
        "pending_messages": [],
        "snapshot_metas": []
    }"#;
    let cp: SessionCheckpoint =
        serde_json::from_str(old_json).expect("must deserialize old format");
    assert_eq!(cp.user_appends, vec!["cmd1", "cmd2"]);
}

/// New format JSON with "user_appends" key deserializes correctly.
#[test]
fn test_persistence_new_user_appends_key() {
    let mut cp = SessionCheckpoint::new("new-key-test".into());
    cp.user_appends = vec!["new1".into(), "new2".into()];
    let json = serde_json::to_string(&cp).unwrap();
    let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.user_appends, vec!["new1", "new2"]);
}

/// Checkpoint serialized JSON must NOT contain "system_injection_appends"
/// (runtime-only field, #[serde(skip)]).
#[test]
fn test_persistence_no_system_injection_appends_in_json() {
    let mut cp = SessionCheckpoint::new("no-inj-test".into());
    cp.system_injection_appends = vec!["runtime".into()];
    let json = serde_json::to_string(&cp).unwrap();
    assert!(
        !json.contains("system_injection_appends"),
        "system_injection_appends must not appear in serialized JSON"
    );
    // Deserialize back — should default to empty
    let restored: SessionCheckpoint = serde_json::from_str(&json).unwrap();
    assert!(restored.system_injection_appends.is_empty());
}

/// Only "user_appends" present in JSON (new format) — deserializes
/// correctly without needing the alias.
#[test]
fn test_persistence_new_format_user_appends_only() {
    let json = r#"{
        "session_id": "new-only",
        "user_appends": ["from_new"],
        "reasoning_mode": "direct",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "ttl_seconds": 604800,
        "status": "active",
        "message_count": 0,
        "reasoning_level": "high",
        "outbound_pending": [],
        "mode_state": {"current_step": 0, "total_steps": 0, "step_messages": [], "is_complete": false},
        "pending_operations": [],
        "pending_tool_failures": [],
        "progress_tool_calls": [],
        "approval_tool_calls": [],
        "plan_references": [],
        "pending_messages": [],
        "snapshot_metas": []
    }"#;
    let cp: SessionCheckpoint = serde_json::from_str(json).unwrap();
    assert_eq!(cp.user_appends, vec!["from_new"]);
}

/// Missing both keys defaults to empty Vec.
#[test]
fn test_persistence_missing_both_keys_defaults_empty() {
    let json = r#"{
        "session_id": "missing-both",
        "reasoning_mode": "direct",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "ttl_seconds": 604800,
        "status": "active",
        "message_count": 0,
        "reasoning_level": "high",
        "outbound_pending": [],
        "mode_state": {"current_step": 0, "total_steps": 0, "step_messages": [], "is_complete": false},
        "pending_operations": [],
        "pending_tool_failures": [],
        "progress_tool_calls": [],
        "approval_tool_calls": [],
        "plan_references": [],
        "pending_messages": [],
        "snapshot_metas": []
    }"#;
    let cp: SessionCheckpoint = serde_json::from_str(json).unwrap();
    assert!(cp.user_appends.is_empty());
}
