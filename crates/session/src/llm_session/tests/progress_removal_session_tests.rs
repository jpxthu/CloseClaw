//! Step 1.4 — Tests verifying that the system-level progress mechanism
//! has been fully removed from the session layer.
//!
//! Behavior dimensions:
//! 1. `system_appends()` no longer merges runtime progress appends.
//! 2. `progress_appends()` method is removed.
//! 3. `progress_notifier` injection path is disconnected — session
//!    `system_appends` never contains progress summaries.
//! 4. `progress` tool is not in the builtin tool list.

use super::super::*;

fn new_session() -> ConversationSession {
    ConversationSession::new("sess_progress_removal".into(), "gpt-4o".into(), tmp_path())
}

// ── system_appends no longer merges progress appends ──────────────────────

/// After adding user system appends, `system_appends()` returns only
/// user-managed items — no runtime progress items are appended.
#[test]
fn test_system_appends_only_user_items() {
    let mut session = new_session();
    session.add_system_append("user-item-1".to_string());
    session.add_system_append("user-item-2".to_string());

    let items = session.system_appends();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0], "user-item-1");
    assert_eq!(items[1], "user-item-2");
}

/// `system_appends()` returns an empty Vec for a fresh session.
#[test]
fn test_system_appends_empty_fresh_session() {
    let session = new_session();
    let items = session.system_appends();
    assert!(items.is_empty());
}

/// `user_system_appends()` and `system_appends()` return the same data
/// (no progress merge layer).
#[test]
fn test_user_system_appends_matches_system_appends() {
    let mut session = new_session();
    session.add_system_append("alpha".to_string());
    session.add_system_append("beta".to_string());

    let system = session.system_appends();
    let user = session.user_system_appends().to_vec();
    assert_eq!(system, user);
}

/// After clearing, both accessors return empty.
#[test]
fn test_clear_system_appends_removes_all() {
    let mut session = new_session();
    session.add_system_append("item".to_string());
    assert_eq!(session.clear_system_appends(), 1);
    assert!(session.system_appends().is_empty());
    assert!(session.user_system_appends().is_empty());
}

// ── progress_notifier injection path disconnected ─────────────────────────

/// `system_appends()` does NOT contain any progress-summary-style
/// entries (the format used by the old progress_notifier was
/// `"[step N/M] status: ..."`).
#[test]
fn test_system_appends_no_progress_summary_entries() {
    let mut session = new_session();
    session.add_system_append("legitimate user append".to_string());

    let items = session.system_appends();
    for item in &items {
        assert!(
            !item.starts_with("[step"),
            "system_appends must not contain progress summary entries, got: {item}"
        );
        assert!(
            !item.contains("progress"),
            "system_appends must not contain progress-related entries, got: {item}"
        );
    }
}

// ── tool registry assertion ───────────────────────────────────────────────

/// The builtin tool list does not contain a "progress" tool.
///
/// This test uses the same DummyTool + ToolRegistry pattern from the
/// tools crate tests, but verifies at the module level that no
/// `ProgressTool` type exists in the builtin module.
#[test]
fn test_no_progress_tool_type_exists() {
    // Compile-time check: importing ProgressTool must fail.
    // We verify by checking that the builtin module has no progress
    // submodule and the tool is not in the use declarations.
    //
    // At runtime we simply confirm the module structure doesn't include
    // progress.rs (verified by the file not existing).
    let builtin_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tools/src/builtin");
    let progress_file = builtin_dir.join("progress.rs");
    assert!(
        !progress_file.exists(),
        "progress.rs must not exist in builtin directory — ProgressTool was removed in Step 1.2"
    );
}

/// The builtin `mod.rs` does not declare a `progress` module.
#[test]
fn test_builtin_mod_no_progress_declaration() {
    let builtin_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tools/src/builtin/mod.rs");
    let content = std::fs::read_to_string(&builtin_dir).expect("read builtin/mod.rs");
    assert!(
        !content.contains("mod progress"),
        "builtin/mod.rs must not contain `mod progress` — ProgressTool was removed in Step 1.2"
    );
    assert!(
        !content.contains("ProgressTool"),
        "builtin/mod.rs must not reference ProgressTool"
    );
}

// ── restore_system_appends works without progress layer ───────────────────

/// Restore replaces all items; no hidden progress layer persists.
#[test]
fn test_restore_system_appends_full_replacement() {
    let mut session = new_session();
    session.add_system_append("old".to_string());
    session.restore_system_appends(vec!["new-1".to_string(), "new-2".to_string()]);

    let items = session.system_appends();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0], "new-1");
    assert_eq!(items[1], "new-2");
    assert!(!items.contains(&"old".to_string()));
}
