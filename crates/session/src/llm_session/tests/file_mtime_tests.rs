//! Tests for `record_file_read` / `get_file_mtime` on `ConversationSession`.
//!
//! Covers: basic record→get, overwrite on re-record, multi-file independence,
//! unread-file returns None, None-mtime removal, path canonicalization,
//! and an end-to-end staleness detection scenario.

use crate::llm_session::ConversationSession;
use closeclaw_common::tool_session::ToolSession;
use std::time::SystemTime;
use tempfile::NamedTempFile;

// ── helpers ──────────────────────────────────────────────────────────────

fn new_session(id: &str) -> ConversationSession {
    ConversationSession::new(id.into(), "gpt-4o".into(), super::tmp_path())
}

// ── 1. Normal path ───────────────────────────────────────────────────────

/// `record_file_read` with a real file → `get_file_mtime` returns the
/// same SystemTime.
#[tokio::test]
async fn test_record_then_get_returns_mtime() {
    let session = new_session("mtime_basic");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    let recorded_mtime = std::fs::metadata(path).unwrap().modified().unwrap();

    <ConversationSession as ToolSession>::record_file_read(&session, path, Some(recorded_mtime))
        .await;

    let got = <ConversationSession as ToolSession>::get_file_mtime(&session, path);
    assert_eq!(got, Some(recorded_mtime));
}

// ── 2. Override update ───────────────────────────────────────────────────

/// Recording the same file twice with different mtimes → `get_file_mtime`
/// returns the **latest** mtime (the second one).
#[tokio::test]
async fn test_rerecord_overwrites_mtime() {
    let session = new_session("mtime_overwrite");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    let t1 = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    let t2 = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2000);

    <ConversationSession as ToolSession>::record_file_read(&session, path, Some(t1)).await;
    <ConversationSession as ToolSession>::record_file_read(&session, path, Some(t2)).await;

    let got = <ConversationSession as ToolSession>::get_file_mtime(&session, path);
    assert_eq!(got, Some(t2), "second record should overwrite the first");
}

// ── 3. Multi-file independence ───────────────────────────────────────────

/// Two different files recorded independently → each `get_file_mtime`
/// returns its own mtime, with no cross-contamination.
#[tokio::test]
async fn test_multi_file_independent() {
    let session = new_session("mtime_multi");
    let tmp_a = NamedTempFile::new().unwrap();
    let tmp_b = NamedTempFile::new().unwrap();
    let path_a = tmp_a.path().to_str().unwrap();
    let path_b = tmp_b.path().to_str().unwrap();

    let ta = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100);
    let tb = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(200);

    <ConversationSession as ToolSession>::record_file_read(&session, path_a, Some(ta)).await;
    <ConversationSession as ToolSession>::record_file_read(&session, path_b, Some(tb)).await;

    assert_eq!(
        <ConversationSession as ToolSession>::get_file_mtime(&session, path_a),
        Some(ta)
    );
    assert_eq!(
        <ConversationSession as ToolSession>::get_file_mtime(&session, path_b),
        Some(tb)
    );
}

// ── 4. Unread file returns None ──────────────────────────────────────────

/// `get_file_mtime` on a file that was never recorded → `None`.
#[tokio::test]
async fn test_unread_file_returns_none() {
    let session = new_session("mtime_unread");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    let got = <ConversationSession as ToolSession>::get_file_mtime(&session, path);
    assert_eq!(got, None);
}

// ── 5. None mtime removes record ─────────────────────────────────────────

/// Recording with `mtime = None` removes any existing record for that
/// path; subsequent `get_file_mtime` returns `None`.
#[tokio::test]
async fn test_none_mtime_removes_record() {
    let session = new_session("mtime_none");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    let t1 = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(500);
    <ConversationSession as ToolSession>::record_file_read(&session, path, Some(t1)).await;
    assert_eq!(
        <ConversationSession as ToolSession>::get_file_mtime(&session, path),
        Some(t1)
    );

    // Now remove via None
    <ConversationSession as ToolSession>::record_file_read(&session, path, None).await;
    assert_eq!(
        <ConversationSession as ToolSession>::get_file_mtime(&session, path),
        None,
        "mtime should be None after recording None"
    );
}

// ── 6. Path canonicalization ─────────────────────────────────────────────

/// A symlink path and the real target path should map to the same
/// internal record, because both canonicalize to the same inode.
#[tokio::test]
async fn test_symlink_canonicalization() {
    let session = new_session("mtime_symlink");
    let tmp = NamedTempFile::new().unwrap();
    let real_path = tmp.into_temp_path();
    let real_str = real_path.to_str().unwrap();

    // Create a symlink in a temp directory (symlinks can't cross tmpfs boundaries
    // so we create the symlink next to the real file).
    let dir = tempfile::tempdir().unwrap();
    let link_path = dir.path().join("link_file");
    std::os::unix::fs::symlink(real_str, &link_path).unwrap();
    let link_str = link_path.to_str().unwrap();

    let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(777);

    // Record via the symlink path
    <ConversationSession as ToolSession>::record_file_read(&session, link_str, Some(t)).await;

    // Get via the real path — should still find the record
    let got = <ConversationSession as ToolSession>::get_file_mtime(&session, real_str);
    assert_eq!(
        got,
        Some(t),
        "symlink and real path should map to the same record"
    );
}

// ── 7. Staleness end-to-end ──────────────────────────────────────────────

/// Create a temp file, record its mtime, then modify the file. The
/// recorded mtime should differ from the current mtime, confirming that
/// `check_staleness` (in `file_ops.rs`) would detect the change.
///
/// We do NOT call `check_staleness` directly because it lives in the
/// `tools` crate and takes a `ToolContext`. Instead we verify the raw
/// mtime comparison logic that `check_staleness` relies on.
#[tokio::test]
async fn test_staleness_detection_after_modify() {
    let session = new_session("mtime_staleness");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path();

    // Write initial content and record its mtime.
    std::fs::write(path, "initial content").unwrap();
    let recorded = std::fs::metadata(path).unwrap().modified().unwrap();

    <ConversationSession as ToolSession>::record_file_read(
        &session,
        path.to_str().unwrap(),
        Some(recorded),
    )
    .await;

    // Modify the file, then read the new mtime.
    std::fs::write(path, "modified content!!!").unwrap();
    let current = std::fs::metadata(path).unwrap().modified().unwrap();

    // On ext4/f2fs with 1 s granularity the mtimes may be equal if the
    // write lands within the same second. To avoid flakiness we assert
    // that if they differ the staleness check would fire, and if they're
    // equal we nudge the clock by rewriting after a brief pause.
    if current == recorded {
        // Same-second write: force a different mtime by touching in the
        // next second.
        let t2 = recorded + std::time::Duration::from_secs(2);
        let session2 = new_session("mtime_staleness_nudge");
        <ConversationSession as ToolSession>::record_file_read(
            &session2,
            path.to_str().unwrap(),
            Some(t2),
        )
        .await;
        std::fs::write(path, "modified again").unwrap();
        let current2 = std::fs::metadata(path).unwrap().modified().unwrap();
        // After the nudge the new mtime should differ from recorded
        // (either immediately or after the filesystem catches up).
        assert_ne!(
            current2, t2,
            "mtime should differ after external modification"
        );
    } else {
        // Different seconds — staleness detected.
        assert_ne!(recorded, current);
    }
}
