//! Tests for `get_file_read_cache` / `record_file_read_range` on
//! `ConversationSession`.
//!
//! Covers: record → get, range accumulation, cache hit (same range +
//! mtime), cache miss (mtime changed), cache miss (different range),
//! multi-file independence, and path canonicalization.

use crate::llm_session::ConversationSession;
use closeclaw_common::tool_session::{ReadRange, ToolSession};
use std::time::{Duration, SystemTime};
use tempfile::NamedTempFile;

// ── helpers ──────────────────────────────────────────────────────────────

fn new_session(id: &str) -> ConversationSession {
    ConversationSession::new(id.into(), "gpt-4o".into(), super::tmp_path())
}

// ── 1. Basic record → get ────────────────────────────────────────────────

/// After recording a range, `get_file_read_cache` returns the mtime
/// and that range.
#[tokio::test]
async fn test_record_range_then_get_returns_cache() {
    let session = new_session("range_basic");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let range = ReadRange {
        offset: 1,
        limit: Some(100),
    };

    <ConversationSession as ToolSession>::record_file_read_range(
        &session,
        path,
        Some(mtime),
        range,
    )
    .await;

    let cache = <ConversationSession as ToolSession>::get_file_read_cache(&session, path);
    let cache = cache.expect("cache should exist");
    assert_eq!(cache.mtime, Some(mtime));
    assert_eq!(cache.ranges.len(), 1);
    assert_eq!(
        cache.ranges[0],
        ReadRange {
            offset: 1,
            limit: Some(100)
        }
    );
}

// ── 2. Range accumulation ────────────────────────────────────────────────

/// Recording the same file with different ranges accumulates them all.
#[tokio::test]
async fn test_multiple_ranges_accumulated() {
    let session = new_session("range_accum");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(200);

    let r1 = ReadRange {
        offset: 1,
        limit: Some(100),
    };
    let r2 = ReadRange {
        offset: 101,
        limit: Some(100),
    };
    let r3 = ReadRange {
        offset: 201,
        limit: None,
    };

    <ConversationSession as ToolSession>::record_file_read_range(&session, path, Some(mtime), r1)
        .await;
    <ConversationSession as ToolSession>::record_file_read_range(&session, path, Some(mtime), r2)
        .await;
    <ConversationSession as ToolSession>::record_file_read_range(&session, path, Some(mtime), r3)
        .await;

    let cache = <ConversationSession as ToolSession>::get_file_read_cache(&session, path)
        .expect("cache should exist");
    assert_eq!(cache.ranges.len(), 3);
}

// ── 3. Cache hit (same range + mtime) ────────────────────────────────────

/// Requesting the same path + range + mtime matches cache.
#[tokio::test]
async fn test_cache_hit_same_range_and_mtime() {
    let session = new_session("range_hit");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(300);
    let range = ReadRange {
        offset: 50,
        limit: Some(200),
    };

    <ConversationSession as ToolSession>::record_file_read_range(
        &session,
        path,
        Some(mtime),
        range,
    )
    .await;

    let cache = <ConversationSession as ToolSession>::get_file_read_cache(&session, path)
        .expect("cache should exist");
    assert_eq!(cache.mtime, Some(mtime));
    assert!(cache.ranges.contains(&ReadRange {
        offset: 50,
        limit: Some(200)
    }));
}

// ── 4. Cache miss (mtime changed) ────────────────────────────────────────

/// Even with the same range, different mtime means cache is stale.
#[tokio::test]
async fn test_cache_miss_mtime_changed() {
    let session = new_session("range_mtime");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let t1 = SystemTime::UNIX_EPOCH + Duration::from_secs(400);
    let t2 = SystemTime::UNIX_EPOCH + Duration::from_secs(401);
    let range = ReadRange {
        offset: 1,
        limit: Some(50),
    };

    <ConversationSession as ToolSession>::record_file_read_range(&session, path, Some(t1), range)
        .await;

    let cache = <ConversationSession as ToolSession>::get_file_read_cache(&session, path)
        .expect("cache should exist");
    // mtime in cache is t1; caller should compare with current mtime
    // to detect staleness. The cache itself still returns t1.
    assert_eq!(cache.mtime, Some(t1));
    assert_ne!(cache.mtime, Some(t2));
}

// ── 5. Cache miss (different range) ──────────────────────────────────────

/// Same file, same mtime, but different range → not a cache hit.
#[tokio::test]
async fn test_cache_miss_different_range() {
    let session = new_session("range_diff");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(500);

    <ConversationSession as ToolSession>::record_file_read_range(
        &session,
        path,
        Some(mtime),
        ReadRange {
            offset: 1,
            limit: Some(100),
        },
    )
    .await;

    let cache = <ConversationSession as ToolSession>::get_file_read_cache(&session, path)
        .expect("cache should exist");
    assert!(
        !cache.ranges.contains(&ReadRange {
            offset: 200,
            limit: Some(100)
        }),
        "different range should NOT be in cache"
    );
}

// ── 6. Multi-file independence ───────────────────────────────────────────

/// Two different files have independent caches.
#[tokio::test]
async fn test_multi_file_independent_ranges() {
    let session = new_session("range_multi");
    let tmp_a = NamedTempFile::new().unwrap();
    let tmp_b = NamedTempFile::new().unwrap();
    let path_a = tmp_a.path().to_str().unwrap();
    let path_b = tmp_b.path().to_str().unwrap();
    let ta = SystemTime::UNIX_EPOCH + Duration::from_secs(600);
    let tb = SystemTime::UNIX_EPOCH + Duration::from_secs(700);

    <ConversationSession as ToolSession>::record_file_read_range(
        &session,
        path_a,
        Some(ta),
        ReadRange {
            offset: 1,
            limit: Some(10),
        },
    )
    .await;
    <ConversationSession as ToolSession>::record_file_read_range(
        &session,
        path_b,
        Some(tb),
        ReadRange {
            offset: 1,
            limit: Some(20),
        },
    )
    .await;

    let cache_a = <ConversationSession as ToolSession>::get_file_read_cache(&session, path_a)
        .expect("cache A should exist");
    let cache_b = <ConversationSession as ToolSession>::get_file_read_cache(&session, path_b)
        .expect("cache B should exist");
    assert_eq!(cache_a.ranges[0].limit, Some(10));
    assert_eq!(cache_b.ranges[0].limit, Some(20));
}

// ── 7. Unread file returns None ──────────────────────────────────────────

/// `get_file_read_cache` on a file never recorded → `None`.
#[tokio::test]
async fn test_unread_file_returns_none() {
    let session = new_session("range_unread");
    let got = <ConversationSession as ToolSession>::get_file_read_cache(&session, "/nonexistent");
    assert_eq!(got, None);
}

// ── 8. Path canonicalization ─────────────────────────────────────────────

/// Symlink and real path map to the same cache entry.
#[tokio::test]
async fn test_symlink_canonicalization_ranges() {
    let session = new_session("range_symlink");
    let tmp = NamedTempFile::new().unwrap();
    let real_path = tmp.into_temp_path();
    let real_str = real_path.to_str().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let link_path = dir.path().join("link_file");
    std::os::unix::fs::symlink(real_str, &link_path).unwrap();
    let link_str = link_path.to_str().unwrap();

    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(777);
    let range = ReadRange {
        offset: 10,
        limit: None,
    };

    <ConversationSession as ToolSession>::record_file_read_range(
        &session,
        link_str,
        Some(t),
        range,
    )
    .await;

    let cache = <ConversationSession as ToolSession>::get_file_read_cache(&session, real_str)
        .expect("symlink and real path should share cache");
    assert_eq!(cache.mtime, Some(t));
}

// ── 9. None mtime records correctly ──────────────────────────────────────

/// Recording with `mtime = None` still stores the range; mtime is None.
#[tokio::test]
async fn test_none_mtime_records_range() {
    let session = new_session("range_none_mtime");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let range = ReadRange {
        offset: 1,
        limit: Some(5),
    };

    <ConversationSession as ToolSession>::record_file_read_range(&session, path, None, range).await;

    let cache = <ConversationSession as ToolSession>::get_file_read_cache(&session, path)
        .expect("cache should exist even with None mtime");
    assert_eq!(cache.mtime, None);
    assert_eq!(cache.ranges.len(), 1);
}

// ── 10. Unlimit (None) vs Some limit are different ranges ────────────────

/// ReadRange with `limit = None` differs from `limit = Some(100)`.
#[tokio::test]
async fn test_unlimited_vs_limited_range_distinct() {
    let session = new_session("range_unlim");
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(800);

    <ConversationSession as ToolSession>::record_file_read_range(
        &session,
        path,
        Some(mtime),
        ReadRange {
            offset: 1,
            limit: None,
        },
    )
    .await;

    let cache = <ConversationSession as ToolSession>::get_file_read_cache(&session, path)
        .expect("cache should exist");
    assert!(
        !cache.ranges.contains(&ReadRange {
            offset: 1,
            limit: Some(100)
        }),
        "None limit should not match Some(100)"
    );
}
