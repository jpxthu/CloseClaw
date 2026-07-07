//! Unit tests for [`PlanStateNotifier`] implementation on
//! [`ConversationSession`] (Step 1.3).

use super::super::*;
use super::PROGRESS_APPEND_PREFIX;
use closeclaw_common::PlanStateNotifier;

// ── helpers ──────────────────────────────────────────────────────────────

fn new_session() -> ConversationSession {
    ConversationSession::new("sess_progress".into(), "gpt-4o".into(), tmp_path())
}

// ── test_progress_notifier_replaces_existing ─────────────────────────────

#[tokio::test]
async fn test_progress_notifier_replaces_existing() {
    let session = new_session();

    // First update — appends a new entry.
    session.on_progress_changed("Step 1/3: completed").await;
    let appends = session.system_appends();
    assert_eq!(appends.len(), 1);
    assert!(appends[0].starts_with(PROGRESS_APPEND_PREFIX));
    assert!(appends[0].contains("Step 1/3: completed"));

    // Second update — replaces the existing entry (no duplicate).
    session.on_progress_changed("Step 2/3: in_progress").await;
    let appends = session.system_appends();
    assert_eq!(
        appends
            .iter()
            .filter(|s| s.starts_with(PROGRESS_APPEND_PREFIX))
            .count(),
        1,
        "should have exactly one progress entry after replacement"
    );
    assert!(appends[0].contains("Step 2/3: in_progress"));
}

// ── test_progress_notifier_empty_removes_entry ───────────────────────────

#[tokio::test]
async fn test_progress_notifier_empty_removes_entry() {
    let session = new_session();

    session.on_progress_changed("Step 1/3: done").await;
    assert_eq!(session.system_appends().len(), 1);

    // Empty summary removes the progress entry.
    session.on_progress_changed("").await;
    let appends = session.system_appends();
    assert!(
        appends
            .iter()
            .all(|s| !s.starts_with(PROGRESS_APPEND_PREFIX)),
        "progress entry should be removed on empty summary"
    );
}

// ── test_progress_notifier_does_not_touch_user_appends ───────────────────

#[tokio::test]
async fn test_progress_notifier_does_not_touch_user_appends() {
    let mut session = new_session();
    session.add_system_append("user-note-1".to_string());
    session.add_system_append("user-note-2".to_string());

    session.on_progress_changed("Step 1/3: done").await;

    let appends = session.system_appends();
    // User appends + 1 progress = 3 total
    assert_eq!(appends.len(), 3);
    // User appends are preserved.
    assert!(appends.iter().any(|s| s == "user-note-1"));
    assert!(appends.iter().any(|s| s == "user-note-2"));
    // Progress entry is appended at end.
    assert!(appends.last().unwrap().starts_with(PROGRESS_APPEND_PREFIX));
}

// ── test_progress_notifier_user_appends_stable_after_progress ────────────

#[tokio::test]
async fn test_progress_notifier_user_appends_stable_after_progress() {
    let mut session = new_session();
    session.add_system_append("user-note".to_string());

    session.on_progress_changed("Step 1/3: done").await;
    session.on_progress_changed("Step 2/3: done").await;
    session.on_progress_changed("Step 3/3: done").await;

    // User append is still present.
    let appends = session.system_appends();
    assert!(appends.iter().any(|s| s == "user-note"));
    // Only one progress entry (replaced each time).
    assert_eq!(
        appends
            .iter()
            .filter(|s| s.starts_with(PROGRESS_APPEND_PREFIX))
            .count(),
        1
    );
}

// ── test_progress_notifier_empty_on_empty_session ────────────────────────

#[tokio::test]
async fn test_progress_notifier_empty_on_empty_session() {
    let session = new_session();
    assert!(session.system_appends().is_empty());

    // Setting progress, then clearing — back to empty.
    session.on_progress_changed("Step 1/3: done").await;
    session.on_progress_changed("").await;
    assert!(session.system_appends().is_empty());
}

// ── test_progress_notifier_truncation ────────────────────────────────────

#[tokio::test]
async fn test_progress_notifier_truncation() {
    use super::super::APPEND_SECTION_MAX_LEN;

    let session = new_session();

    // Content at the limit is stored unchanged.
    let _at_limit = format!(
        "{}{}",
        PROGRESS_APPEND_PREFIX,
        "x".repeat(APPEND_SECTION_MAX_LEN)
    );
    session
        .on_progress_changed(&"x".repeat(APPEND_SECTION_MAX_LEN))
        .await;
    let appends = session.system_appends();
    assert_eq!(appends.len(), 1);
    assert_eq!(appends[0].chars().count(), APPEND_SECTION_MAX_LEN);

    // Content over the limit is truncated.
    let over_limit = "y".repeat(APPEND_SECTION_MAX_LEN + 10);
    session.on_progress_changed(&over_limit).await;
    let appends = session.system_appends();
    assert_eq!(appends.len(), 1);
    assert_eq!(appends[0].chars().count(), APPEND_SECTION_MAX_LEN);
}

// ── test_user_system_appends_excludes_progress ───────────────────────────

#[tokio::test]
async fn test_user_system_appends_excludes_progress() {
    let mut session = new_session();
    session.add_system_append("user-note".to_string());
    session.on_progress_changed("Step 1/3: done").await;

    // user_system_appends returns only user-managed items.
    let user = session.user_system_appends();
    assert_eq!(user.len(), 1);
    assert_eq!(user[0], "user-note");

    // system_appends (merged) includes both.
    let all = session.system_appends();
    assert_eq!(all.len(), 2);
}

// ── test_progress_appends_accessor ───────────────────────────────────────

#[tokio::test]
async fn test_progress_appends_accessor() {
    let session = new_session();
    assert!(session.progress_appends().is_empty());

    session.on_progress_changed("Step 1/3: done").await;
    let progress = session.progress_appends();
    assert_eq!(progress.len(), 1);
    assert!(progress[0].contains("Step 1/3: done"));
}
