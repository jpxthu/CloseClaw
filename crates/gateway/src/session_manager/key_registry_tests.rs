//! Tests for `key_registry` rebuild logic.
//!
//! Verifies that `rebuild_key_registry` correctly:
//! - Selects the checkpoint with the latest `last_message_at` per key
//! - Falls back to `created_at` when `last_message_at` is `None`

use super::SessionManager;
use crate::GatewayConfig;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

// ── Mock PersistenceService ────────────────────────────────────────────────

/// In-memory mock for persistence, keyed by session_id.
struct MockPersistence {
    active_sessions: Mutex<Vec<String>>,
    checkpoints: std::sync::Mutex<HashMap<String, SessionCheckpoint>>,
}

impl MockPersistence {
    fn new() -> Self {
        Self {
            active_sessions: Mutex::new(Vec::new()),
            checkpoints: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn with_checkpoint(self, cp: SessionCheckpoint) -> Self {
        self.checkpoints
            .lock()
            .unwrap()
            .insert(cp.session_id.clone(), cp.clone());
        self.active_sessions.lock().unwrap().push(cp.session_id);
        self
    }
}

#[async_trait]
impl PersistenceService for MockPersistence {
    async fn save_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoints.lock().unwrap().get(session_id).cloned())
    }

    async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self.active_sessions.lock().unwrap().clone())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_checkpoint(
    session_id: &str,
    platform: &str,
    peer_id: &str,
    sender_id: Option<&str>,
    account_id: Option<&str>,
    created_at: DateTime<Utc>,
    last_message_at: Option<DateTime<Utc>>,
) -> SessionCheckpoint {
    let mut cp = SessionCheckpoint::new(session_id.to_string());
    cp.platform = Some(platform.to_string());
    cp.peer_id = Some(peer_id.to_string());
    cp.sender_id = sender_id.map(|s| s.to_string());
    cp.account_id = account_id.map(|a| a.to_string());
    cp.created_at = created_at;
    cp.last_message_at = last_message_at;
    cp
}

fn make_manager(mock: Arc<MockPersistence>) -> SessionManager {
    let config = GatewayConfig::default();
    SessionManager::new(&config, Some(mock), None, Default::default())
}

async fn get_key_registry(mgr: &SessionManager) -> HashMap<String, String> {
    mgr.key_registry.read().await.clone()
}

// ── Test 1: latest last_message_at wins ────────────────────────────────────

/// When two sessions share the same routing key, the one with the latest
/// `last_message_at` should be selected.
#[tokio::test]
async fn test_rebuild_selects_latest_last_message_at() {
    let now = Utc::now();
    let old = now - Duration::hours(2);
    let recent = now - Duration::minutes(30);

    // Two sessions with the same routing fields → same hash key
    let cp_old = make_checkpoint(
        "session-old",
        "feishu",
        "chat-1",
        Some("user-1"),
        Some("account-1"),
        old,
        Some(old),
    );
    let cp_new = make_checkpoint(
        "session-new",
        "feishu",
        "chat-1",
        Some("user-1"),
        Some("account-1"),
        recent,
        Some(recent),
    );

    let mock = Arc::new(
        MockPersistence::new()
            .with_checkpoint(cp_old)
            .with_checkpoint(cp_new),
    );
    let mgr = make_manager(mock);

    mgr.rebuild_key_registry().await.unwrap();

    let registry = get_key_registry(&mgr).await;
    assert_eq!(registry.len(), 1, "should have exactly one entry per key");

    let (_, session_id) = registry.iter().next().unwrap();
    assert_eq!(
        session_id, "session-new",
        "should select the session with the latest last_message_at"
    );
}

// ── Test 2: fallback to created_at when last_message_at is None ────────────

/// When `last_message_at` is `None`, the rebuild should fall back to
/// `created_at` for ordering.
#[tokio::test]
async fn test_rebuild_fallback_to_created_at() {
    let now = Utc::now();
    let old = now - Duration::hours(1);
    let recent = now;

    // Session A: last_message_at = None, created_at = old
    let cp_a = make_checkpoint(
        "session-a",
        "feishu",
        "chat-2",
        Some("user-2"),
        Some("account-2"),
        old,
        None, // last_message_at is None → fallback to created_at
    );
    // Session B: last_message_at = None, created_at = recent
    let cp_b = make_checkpoint(
        "session-b",
        "feishu",
        "chat-2",
        Some("user-2"),
        Some("account-2"),
        recent,
        None,
    );

    let mock = Arc::new(
        MockPersistence::new()
            .with_checkpoint(cp_a)
            .with_checkpoint(cp_b),
    );
    let mgr = make_manager(mock);

    mgr.rebuild_key_registry().await.unwrap();

    let registry = get_key_registry(&mgr).await;
    assert_eq!(registry.len(), 1);

    let (_, session_id) = registry.iter().next().unwrap();
    assert_eq!(
        session_id, "session-b",
        "should fall back to created_at and pick the newer one"
    );
}

// ── Test 3: mixed — one has last_message_at, one doesn't ───────────────────

/// When one checkpoint has `last_message_at` and the other does not,
/// the one with `last_message_at` (even if older `created_at`) should win
/// because `last_message_at` takes priority.
#[tokio::test]
async fn test_rebuild_last_message_at_beats_created_at() {
    let now = Utc::now();
    let very_old = now - Duration::days(1);
    let somewhat_old = now - Duration::hours(1);
    let recent = now - Duration::minutes(10);

    // Session A: last_message_at = Some(recent), created_at = very_old
    let cp_a = make_checkpoint(
        "session-a",
        "feishu",
        "chat-3",
        Some("user-3"),
        Some("account-3"),
        very_old,
        Some(recent),
    );
    // Session B: last_message_at = None, created_at = somewhat_old
    // (somewhat_old > very_old, but A wins because its last_message_at is newer)
    let cp_b = make_checkpoint(
        "session-b",
        "feishu",
        "chat-3",
        Some("user-3"),
        Some("account-3"),
        somewhat_old,
        None,
    );

    let mock = Arc::new(
        MockPersistence::new()
            .with_checkpoint(cp_a)
            .with_checkpoint(cp_b),
    );
    let mgr = make_manager(mock);

    mgr.rebuild_key_registry().await.unwrap();

    let registry = get_key_registry(&mgr).await;
    assert_eq!(registry.len(), 1);

    let (_, session_id) = registry.iter().next().unwrap();
    assert_eq!(
        session_id, "session-a",
        "last_message_at should take priority over created_at"
    );
}

// ── Test 4: different routing keys → separate entries ──────────────────────

/// Sessions with different routing fields should produce different keys.
#[tokio::test]
async fn test_rebuild_different_keys() {
    let now = Utc::now();

    let cp1 = make_checkpoint(
        "session-1",
        "feishu",
        "chat-1",
        Some("user-1"),
        Some("account-1"),
        now,
        Some(now),
    );
    let cp2 = make_checkpoint(
        "session-2",
        "feishu",
        "chat-2",
        Some("user-2"),
        Some("account-1"),
        now,
        Some(now),
    );

    let mock = Arc::new(
        MockPersistence::new()
            .with_checkpoint(cp1)
            .with_checkpoint(cp2),
    );
    let mgr = make_manager(mock);

    mgr.rebuild_key_registry().await.unwrap();

    let registry = get_key_registry(&mgr).await;
    assert_eq!(
        registry.len(),
        2,
        "different routing keys should produce separate entries"
    );
}

// ── Test 5: checkpoint with missing platform/peer_id is skipped ────────────

/// Checkpoints without `platform` or `peer_id` should be skipped
/// (can't reconstruct a routing key).
#[tokio::test]
async fn test_rebuild_skips_incomplete_checkpoint() {
    let now = Utc::now();

    // Valid checkpoint
    let cp_valid = make_checkpoint(
        "session-valid",
        "feishu",
        "chat-1",
        Some("user-1"),
        None,
        now,
        Some(now),
    );
    // Incomplete: missing platform
    let mut cp_no_platform = SessionCheckpoint::new("session-no-platform".to_string());
    cp_no_platform.peer_id = Some("chat-1".to_string());
    cp_no_platform.sender_id = Some("user-1".to_string());
    cp_no_platform.platform = None;

    let mock = Arc::new(
        MockPersistence::new()
            .with_checkpoint(cp_valid)
            .with_checkpoint(cp_no_platform),
    );
    let mgr = make_manager(mock);

    mgr.rebuild_key_registry().await.unwrap();

    let registry = get_key_registry(&mgr).await;
    assert_eq!(
        registry.len(),
        1,
        "only the complete checkpoint should be registered"
    );
}

// ── Test 6: empty persistence → empty registry ─────────────────────────────

#[tokio::test]
async fn test_rebuild_empty_registry() {
    let mock = Arc::new(MockPersistence::new());
    let mgr = make_manager(mock);

    mgr.rebuild_key_registry().await.unwrap();

    let registry = get_key_registry(&mgr).await;
    assert!(registry.is_empty(), "empty persistence → empty registry");
}

// ── Test 7: equal last_message_at → first one wins (stable) ────────────────

/// When two checkpoints have identical `last_message_at`, the first one
/// encountered should be kept (stable behavior).
#[tokio::test]
async fn test_rebuild_equal_timestamps_stable() {
    let now = Utc::now();

    let cp_a = make_checkpoint(
        "session-a",
        "feishu",
        "chat-4",
        Some("user-4"),
        None,
        now,
        Some(now),
    );
    let cp_b = make_checkpoint(
        "session-b",
        "feishu",
        "chat-4",
        Some("user-4"),
        None,
        now,
        Some(now),
    );

    let mock = Arc::new(
        MockPersistence::new()
            .with_checkpoint(cp_a)
            .with_checkpoint(cp_b),
    );
    let mgr = make_manager(mock);

    mgr.rebuild_key_registry().await.unwrap();

    let registry = get_key_registry(&mgr).await;
    assert_eq!(registry.len(), 1);

    let (_, session_id) = registry.iter().next().unwrap();
    // The implementation keeps the first encountered when timestamps are equal
    assert_eq!(session_id, "session-a");
}
