//! Unit tests for the per-session append-section (`system_appends`)
//! surface on `ConversationSession` and the corresponding
//! `SessionCheckpoint::system_appends` persistence field.
//!
//! These tests cover Step 1.5 of issue #860. Each test is a 1:1
//! mapping to a bullet in the plan's "Step 1.5：单元测试" section.

use super::super::*;
use closeclaw_session::persistence::SessionCheckpoint;

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

// ── test_system_appends_max_len_truncation ───────────────────────────────

#[test]
fn test_system_appends_max_len_truncation() {
    use super::super::APPEND_SECTION_MAX_LEN;

    let mut session = new_session();

    // Content exactly at the limit is stored unchanged.
    let at_limit = "x".repeat(APPEND_SECTION_MAX_LEN);
    let idx = session.add_system_append(at_limit.clone());
    assert_eq!(idx, 0);
    assert_eq!(
        session.system_appends()[0].chars().count(),
        APPEND_SECTION_MAX_LEN
    );
    assert_eq!(session.system_appends()[0], at_limit);

    // Content one char over the limit is truncated to the limit.
    let over_limit = "y".repeat(APPEND_SECTION_MAX_LEN + 1);
    let idx2 = session.add_system_append(over_limit);
    assert_eq!(idx2, 1);
    assert_eq!(
        session.system_appends()[1].chars().count(),
        APPEND_SECTION_MAX_LEN
    );
    assert!(session.system_appends()[1].chars().all(|c| c == 'y'));

    // Content well over the limit is truncated, not rejected.
    let way_over = "z".repeat(APPEND_SECTION_MAX_LEN * 3);
    session.add_system_append(way_over);
    assert_eq!(
        session.system_appends()[2].chars().count(),
        APPEND_SECTION_MAX_LEN
    );
}

// ── test_system_appends_checkpoint_roundtrip ─────────────────────────────

#[test]
fn test_system_appends_checkpoint_roundtrip() {
    // Build a checkpoint with non-empty system_appends.
    let mut cp = SessionCheckpoint::new("sess_ckpt".to_string());
    cp.system_appends = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];

    // Serialize → deserialize.
    let json = serde_json::to_string(&cp).expect("serialize SessionCheckpoint");
    let restored: SessionCheckpoint =
        serde_json::from_str(&json).expect("deserialize SessionCheckpoint");

    assert_eq!(restored.system_appends.len(), 3);
    assert_eq!(restored.system_appends[0], "alpha");
    assert_eq!(restored.system_appends[1], "beta");
    assert_eq!(restored.system_appends[2], "gamma");
    assert_eq!(restored.system_appends, cp.system_appends);
}

// ── test_system_appends_checkpoint_default_empty ────────────────────────

#[test]
fn test_system_appends_checkpoint_default_empty() {
    // Simulate a pre-#860 checkpoint JSON that has no `system_appends`
    // field. `#[serde(default)]` on the field must make it deserialize
    // to an empty Vec instead of erroring out.
    //
    // We build this by constructing a full valid `SessionCheckpoint`,
    // serializing it, then stripping the `system_appends` key from the
    // resulting JSON. This guarantees the rest of the payload is
    // shape-correct (we don't have to maintain a hand-written JSON
    // literal that mirrors every other required field).
    let mut full = SessionCheckpoint::new("legacy_sess".to_string());
    // Pre-populate other fields so the round-trip mirror is realistic.
    full.message_count = 42;
    full.last_message_at = Some(chrono::Utc::now());

    let mut json: serde_json::Value =
        serde_json::to_value(&full).expect("serialize SessionCheckpoint");

    // Sanity check: the field exists on a freshly-serialized checkpoint
    // (even if empty). This confirms we're testing the right "remove
    // the key" scenario.
    assert!(
        json.get("system_appends").is_some(),
        "freshly serialized checkpoint should contain system_appends key"
    );

    // Remove the `system_appends` key to simulate a pre-#860 file.
    if let Some(obj) = json.as_object_mut() {
        obj.remove("system_appends");
    }

    let legacy_json = serde_json::to_string(&json).expect("re-serialize legacy JSON");
    assert!(
        !legacy_json.contains("system_appends"),
        "stripped JSON must not contain system_appends key, got: {legacy_json}"
    );

    let cp: SessionCheckpoint =
        serde_json::from_str(&legacy_json).expect("legacy JSON must still deserialize");

    assert!(
        cp.system_appends.is_empty(),
        "legacy checkpoint without system_appends must default to empty Vec, got {:?}",
        cp.system_appends
    );
}
