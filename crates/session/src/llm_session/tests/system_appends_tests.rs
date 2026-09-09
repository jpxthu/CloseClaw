//! Unit tests for the per-session append-section (`system_appends`)
//! surface on `ConversationSession` and the corresponding
//! `SessionCheckpoint::system_appends` persistence field.
//!
//! These tests cover Step 1.7 of the system_appends split plan.
//! Each test maps to a bullet in the plan's behavior dimensions.

use super::super::*;
use crate::persistence::SessionCheckpoint;

// ── helpers ──────────────────────────────────────────────────────────────

fn new_session() -> ConversationSession {
    ConversationSession::new("sess_appends".into(), "gpt-4o".into(), tmp_path())
}

// ── test_system_appends_add_and_get ──────────────────────────────────────

#[test]
fn test_system_appends_add_and_get() {
    let mut session = new_session();
    assert!(session.system_appends().is_empty());

    // First add returns 0.
    let i0 = session.add_system_append("first".to_string());
    assert_eq!(i0, 0);

    // Second add returns 1 (sequential, 0-based).
    let i1 = session.add_system_append("second".to_string());
    assert_eq!(i1, 1);

    // Third add returns 2.
    let i2 = session.add_system_append("third".to_string());
    assert_eq!(i2, 2);

    // Order is preserved in insertion order.
    let items = session.system_appends();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], "first");
    assert_eq!(items[1], "second");
    assert_eq!(items[2], "third");
}

// ── test_system_appends_clear ────────────────────────────────────────────

#[test]
fn test_system_appends_clear() {
    // Clearing an empty list returns 0.
    let mut session = new_session();
    assert_eq!(session.clear_system_appends(), 0);
    assert!(session.system_appends().is_empty());

    // Clearing a list with N items returns N and leaves it empty.
    session.add_system_append("a".to_string());
    session.add_system_append("b".to_string());
    session.add_system_append("c".to_string());
    assert_eq!(session.system_appends().len(), 3);

    let n = session.clear_system_appends();
    assert_eq!(n, 3);
    assert!(session.system_appends().is_empty());

    // Clearing twice is idempotent (returns 0 the second time).
    assert_eq!(session.clear_system_appends(), 0);
}

// ── test_system_appends_restore ──────────────────────────────────────────

#[test]
fn test_system_appends_restore() {
    let mut session = new_session();

    // Pre-existing content should be discarded on restore (overwrite,
    // not append).
    session.add_system_append("stale".to_string());
    session.add_system_append("stale2".to_string());
    assert_eq!(session.system_appends().len(), 2);

    session.restore_system_appends(vec!["fresh1".to_string(), "fresh2".to_string()]);

    let items = session.system_appends();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0], "fresh1");
    assert_eq!(items[1], "fresh2");
    // "stale" / "stale2" must not survive the restore.
    assert!(!items.contains(&"stale".to_string()));
    assert!(!items.contains(&"stale2".to_string()));

    // Restoring an empty vec wipes the list.
    session.restore_system_appends(vec![]);
    assert!(session.system_appends().is_empty());
}

// ── test_system_appends_no_truncation ──────────────────────────────────
//
// After the "不截断" fix, `add_system_append` stores content as-is.
// Length limits are enforced by the caller (SystemHandler), not here.

#[test]
fn test_system_appends_no_truncation() {
    let mut session = new_session();

    // Content exactly at the limit is stored unchanged.
    let at_limit = "x".repeat(APPEND_SECTION_MAX_LEN);
    let idx = session.add_system_append(at_limit.clone());
    assert_eq!(idx, 0);
    assert_eq!(session.system_appends()[0], at_limit);

    // Content one char over the limit is stored as-is (no truncation).
    let over_limit = "y".repeat(APPEND_SECTION_MAX_LEN + 1);
    let idx2 = session.add_system_append(over_limit.clone());
    assert_eq!(idx2, 1);
    assert_eq!(session.system_appends()[1], over_limit);
    assert_eq!(
        session.system_appends()[1].chars().count(),
        APPEND_SECTION_MAX_LEN + 1
    );

    // Content well over the limit is stored as-is (no truncation).
    let way_over = "z".repeat(APPEND_SECTION_MAX_LEN * 3);
    let idx3 = session.add_system_append(way_over.clone());
    assert_eq!(idx3, 2);
    assert_eq!(session.system_appends()[2], way_over);
    assert_eq!(
        session.system_appends()[2].chars().count(),
        APPEND_SECTION_MAX_LEN * 3
    );
}

// ── test_system_appends_checkpoint_roundtrip ─────────────────────────────

#[test]
fn test_system_appends_checkpoint_roundtrip() {
    // Build a checkpoint with non-empty system_appends.
    let mut cp = SessionCheckpoint::new("sess_ckpt".to_string());
    cp.user_appends = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];

    // Serialize → deserialize.
    let json = serde_json::to_string(&cp).expect("serialize SessionCheckpoint");
    let restored: SessionCheckpoint =
        serde_json::from_str(&json).expect("deserialize SessionCheckpoint");

    assert_eq!(restored.user_appends.len(), 3);
    assert_eq!(restored.user_appends[0], "alpha");
    assert_eq!(restored.user_appends[1], "beta");
    assert_eq!(restored.user_appends[2], "gamma");
    assert_eq!(restored.user_appends, cp.user_appends);
}

// ── test_add_system_injection_append ───────────────────────────────────

#[test]
fn test_add_system_injection_append() {
    let mut session = new_session();
    assert!(session.system_injection_appends().is_empty());

    let i0 = session.add_system_injection_append("inject0".to_string());
    assert_eq!(i0, 0);

    let i1 = session.add_system_injection_append("inject1".to_string());
    assert_eq!(i1, 1);

    let items = session.system_injection_appends();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0], "inject0");
    assert_eq!(items[1], "inject1");
}

// ── test_independence_clear_user不影响_injection ─────────────────────────

#[test]
fn test_clear_user_appends_does_not_affect_injection() {
    let mut session = new_session();
    session.add_system_append("user1".to_string());
    session.add_system_append("user2".to_string());
    session.add_system_injection_append("inj1".to_string());
    session.add_system_injection_append("inj2".to_string());

    assert_eq!(session.clear_system_appends(), 2);

    // user_appends should be empty
    assert!(session.user_system_appends().is_empty());
    // system_injection_appends should be untouched
    assert_eq!(session.system_injection_appends().len(), 2);
    assert_eq!(session.system_injection_appends()[0], "inj1");
    assert_eq!(session.system_injection_appends()[1], "inj2");
}

// ── test_independence_clear_injection不影响_user ─────────────────────────

#[test]
fn test_clear_injection_appends_does_not_affect_user() {
    let mut session = new_session();
    session.add_system_append("user1".to_string());
    session.add_system_append("user2".to_string());
    session.add_system_injection_append("inj1".to_string());
    session.add_system_injection_append("inj2".to_string());

    assert_eq!(session.clear_system_injection_appends(), 2);

    // system_injection_appends should be empty
    assert!(session.system_injection_appends().is_empty());
    // user_appends should be untouched
    assert_eq!(session.user_system_appends().len(), 2);
    assert_eq!(session.user_system_appends()[0], "user1");
    assert_eq!(session.user_system_appends()[1], "user2");
}

// ── test_merged_output_order ────────────────────────────────────────────

#[test]
fn test_system_appends_returns_user_before_injection() {
    let mut session = new_session();
    session.add_system_append("u1".to_string());
    session.add_system_append("u2".to_string());
    session.add_system_injection_append("i1".to_string());
    session.add_system_injection_append("i2".to_string());

    let merged = session.system_appends();
    assert_eq!(merged.len(), 4);
    assert_eq!(merged[0], "u1");
    assert_eq!(merged[1], "u2");
    assert_eq!(merged[2], "i1");
    assert_eq!(merged[3], "i2");
}

// ── test_boundary_both_empty ────────────────────────────────────────────

#[test]
fn test_system_appends_both_empty() {
    let session = new_session();
    assert!(session.system_appends().is_empty());
    assert!(session.user_system_appends().is_empty());
    assert!(session.system_injection_appends().is_empty());
}

// ── test_boundary_only_user_empty ───────────────────────────────────────

#[test]
fn test_system_appends_only_user_empty() {
    let mut session = new_session();
    session.add_system_injection_append("only_injection".to_string());

    let merged = session.system_appends();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0], "only_injection");
    assert!(session.user_system_appends().is_empty());
}

// ── test_boundary_only_injection_empty ──────────────────────────────────

#[test]
fn test_system_appends_only_injection_empty() {
    let mut session = new_session();
    session.add_system_append("only_user".to_string());

    let merged = session.system_appends();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0], "only_user");
    assert!(session.system_injection_appends().is_empty());
}

// ── test_state_transition_clear_then_add ────────────────────────────────

#[test]
fn test_clear_then_add_user_correct_count() {
    let mut session = new_session();
    session.add_system_append("a".to_string());
    session.add_system_append("b".to_string());
    assert_eq!(session.system_appends().len(), 2);

    let cleared = session.clear_system_appends();
    assert_eq!(cleared, 2);
    assert!(session.system_appends().is_empty());

    // After clear, add again — count starts from 0
    let idx = session.add_system_append("c".to_string());
    assert_eq!(idx, 0);
    assert_eq!(session.system_appends().len(), 1);
    assert_eq!(session.system_appends()[0], "c");
}

#[test]
fn test_clear_then_add_injection_correct_count() {
    let mut session = new_session();
    session.add_system_injection_append("x".to_string());
    session.add_system_injection_append("y".to_string());
    assert_eq!(session.system_injection_appends().len(), 2);

    let cleared = session.clear_system_injection_appends();
    assert_eq!(cleared, 2);
    assert!(session.system_injection_appends().is_empty());

    let idx = session.add_system_injection_append("z".to_string());
    assert_eq!(idx, 0);
    assert_eq!(session.system_injection_appends().len(), 1);
    assert_eq!(session.system_injection_appends()[0], "z");
}

// ── test_clear_injection_idempotent ─────────────────────────────────────

#[test]
fn test_clear_injection_appends_empty_returns_zero() {
    let mut session = new_session();
    assert_eq!(session.clear_system_injection_appends(), 0);
    // Double-clear is idempotent
    assert_eq!(session.clear_system_injection_appends(), 0);
}

// ── test_checkpoint_roundtrip_no_injection_field ────────────────────────

#[test]
fn test_checkpoint_no_system_injection_appends_field() {
    // system_injection_appends is #[serde(skip)] — must NOT appear in JSON
    let mut cp = SessionCheckpoint::new("sess_no_inj".to_string());
    cp.system_injection_appends = vec!["runtime_only".to_string()];

    let json = serde_json::to_string(&cp).expect("serialize");
    assert!(
        !json.contains("system_injection_appends"),
        "system_injection_appends must not appear in serialized JSON"
    );

    // Deserialize — system_injection_appends should default to empty
    let restored: SessionCheckpoint = serde_json::from_str(&json).expect("deserialize");
    assert!(
        restored.system_injection_appends.is_empty(),
        "system_injection_appends must default to empty after deserialization"
    );
}

// ── test_checkpoint_old_format_system_appends_alias ─────────────────────

#[test]
fn test_checkpoint_old_format_system_appends_deserializes_to_user_appends() {
    // Simulate old checkpoint JSON with only "system_appends" key
    // (no "user_appends" — serde alias maps it to user_appends)
    let old_json = r#"{
        "session_id": "old_sess",
        "system_appends": ["old_a", "old_b"],
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
        serde_json::from_str(old_json).expect("old format must deserialize");
    assert_eq!(cp.user_appends, vec!["old_a", "old_b"]);
}

// ── test_checkpoint_new_format_user_appends ─────────────────────────────

#[test]
fn test_checkpoint_new_format_user_appends_deserializes_correctly() {
    let mut cp = SessionCheckpoint::new("new_sess".to_string());
    cp.user_appends = vec!["new_a".to_string(), "new_b".to_string()];

    let json = serde_json::to_string(&cp).expect("serialize");
    let restored: SessionCheckpoint = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(restored.user_appends, vec!["new_a", "new_b"]);
}

// ── test_checkpoint_default_empty ───────────────────────────────────────

#[test]
fn test_system_appends_checkpoint_default_empty() {
    // Simulate a pre-#860 checkpoint JSON that has no `system_appends`
    // field. `#[serde(default)]` on the field must make it deserialize
    // to an empty Vec instead of erroring out.
    let mut full = SessionCheckpoint::new("legacy_sess".to_string());
    full.message_count = 42;
    full.last_message_at = Some(chrono::Utc::now());

    let mut json: serde_json::Value =
        serde_json::to_value(&full).expect("serialize SessionCheckpoint");

    assert!(
        json.get("user_appends").is_some(),
        "freshly serialized checkpoint should contain user_appends key"
    );

    // Remove the `user_appends` key to simulate a pre-#860 file.
    if let Some(obj) = json.as_object_mut() {
        obj.remove("user_appends");
    }

    let legacy_json = serde_json::to_string(&json).expect("re-serialize legacy JSON");
    assert!(
        !legacy_json.contains("user_appends"),
        "stripped JSON must not contain user_appends key, got: {legacy_json}"
    );

    let cp: SessionCheckpoint =
        serde_json::from_str(&legacy_json).expect("legacy JSON must still deserialize");

    assert!(
        cp.user_appends.is_empty(),
        "legacy checkpoint without system_appends must default to empty Vec, got {:?}",
        cp.user_appends
    );
}
