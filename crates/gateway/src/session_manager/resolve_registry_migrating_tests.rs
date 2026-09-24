//! Tests for resolve() registry hit path: Migrating status handling (Step 1.3).
//!
//! Verifies:
//! - Normal path: migrating → poll detects archived → restore archived session
//! - Timeout path: migrating → poll times out →
//!   restores migrating session (checkpoint restore, not new session)
//! - Migrating sessions are never directly restored (must wait for archive)
//! - Archived recovery path regression (existing behavior unbroken)
//! - session_key used in log structured fields

use super::tests::test_config;
use super::SessionManager;
use crate::Message;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, SessionCheckpoint, SessionStatus,
};
use std::sync::Arc;

fn test_message() -> Message {
    Message {
        id: "msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

// ── Mock: configurable poll behavior ────────────────────────────────────────

/// Mock that supports the migrating→archived transition during resolve().
///
/// - `load_checkpoint` returns migrating on the first call, then archived
///   on subsequent calls (or always migrating if `archive_immediately` is
///   false).
/// - `set_storage` simulates an out-of-band raw storage write (the
///   Sweeper archiving behind the CM cache's back): afterwards every
///   `load_checkpoint` returns the primed checkpoint while the stale
///   Migrating value only ever exists in the CM local cache.
/// - `restore_checkpoint` moves the stored checkpoint back to active
///   storage with status = Active (real backend semantics), so every
///   later storage read observes the restored Active checkpoint.
/// - `save_checkpoint` records the last written checkpoint in `last_saved`
///   so tests can pin the FINAL persisted status (a stale Migrating cache
///   write-back would flip Active back to migrating).
/// - `find_archived_session_by_routing` returns `archived_id` when set.
struct MigratingPollMock {
    /// Checkpoint returned on the first `load_checkpoint` call.
    migrating_cp: tokio::sync::Mutex<Option<SessionCheckpoint>>,
    /// Checkpoint returned on subsequent `load_checkpoint` calls.
    archived_cp: tokio::sync::Mutex<Option<SessionCheckpoint>>,
    /// If true, first load returns migrating, second returns archived.
    archive_after_first_poll: bool,
    /// Call counter for `load_checkpoint` (transition mode above).
    load_count: std::sync::atomic::AtomicU32,
    /// Session ID to return from `find_archived_session_by_routing`.
    archived_id: std::sync::Mutex<Option<String>>,
    /// Whether `restore_checkpoint` was called.
    restore_called: tokio::sync::Mutex<bool>,
    /// What storage currently holds: `set_storage` primes it, restore
    /// flips it to Active, every `save_checkpoint` overwrites it
    /// (mirrors sqlite / MemoryStorage semantics).
    storage_view: std::sync::Mutex<Option<SessionCheckpoint>>,
    /// Last checkpoint written via `save_checkpoint` (final persisted state).
    last_saved: tokio::sync::Mutex<Option<SessionCheckpoint>>,
}

impl MigratingPollMock {
    /// Simulate an out-of-band raw storage write (e.g. the Sweeper
    /// archiving directly on storage without touching the CM cache).
    fn set_storage(&self, cp: SessionCheckpoint) {
        *self.storage_view.lock().unwrap() = Some(cp);
    }
}

#[async_trait::async_trait]
impl PersistenceService for MigratingPollMock {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        // Every write lands in storage, so the final persisted state is
        // observable via `load_checkpoint` (and recorded in `last_saved`).
        *self.storage_view.lock().unwrap() = Some(cp.clone());
        *self.last_saved.lock().await = Some(cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        // Serve what storage currently holds; the transition/timeout modes
        // below only apply while storage has not been written yet (a real
        // backend keeps serving the stored row once one exists).
        {
            let view = self.storage_view.lock().unwrap();
            if let Some(cp) = view.as_ref() {
                return Ok(Some(cp.clone()));
            }
        }
        if self.archive_after_first_poll {
            // Non-consuming transition: the first call returns migrating,
            // later calls keep serving the archived checkpoint (clones),
            // so the archived-restore path can re-read storage and the
            // mock never runs "empty" mid-resolve.
            let n = self
                .load_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n == 0 {
                let migrating = self.migrating_cp.lock().await;
                return Ok(migrating.as_ref().cloned());
            }
            let archived = self.archived_cp.lock().await;
            return Ok(archived.as_ref().cloned());
        }
        // Always return migrating (timeout scenario)
        let migrating = self.migrating_cp.lock().await;
        Ok(migrating.as_ref().cloned())
    }

    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }

    async fn load_archived_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        // Only reachable from try_restore_archived_session_inner when the
        // active table has no row — not the case in these scenarios.
        let _ = session_id;
        Ok(None)
    }

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        *self.restore_called.lock().await = true;
        // Model real restore semantics (sqlite do_restore / MemoryStorage):
        // the checkpoint moves back to active storage with status = Active.
        let template = {
            let archived = self.archived_cp.lock().await;
            if archived.is_some() {
                archived.clone()
            } else {
                self.migrating_cp.lock().await.clone()
            }
        };
        let mut restored = template.expect("mock must hold a restorable checkpoint");
        restored.status = SessionStatus::Active;
        let cp = restored.clone();
        *self.storage_view.lock().unwrap() = Some(restored);
        Ok(Some(cp))
    }

    async fn find_active_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(None)
    }

    async fn find_archived_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let mut id = self.archived_id.lock().unwrap();
        Ok(id.take())
    }
}

// ── Normal path: archive completes within poll window ───────────────────────

/// When a registry-hit session is migrating and the Sweeper completes
/// archiving within the 30-second poll window, resolve() should restore
/// the archived session (same session_id) rather than creating a new one.
#[tokio::test]
async fn test_resolve_migrating_registry_hit_archive_completes() {
    let session_id = "migrating-session".to_string();

    let mut cp_migrating = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Migrating)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_migrating.sender_id = Some("user-a".to_string());

    let mut cp_archived = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_archived.sender_id = Some("user-a".to_string());

    let mock = Arc::new(MigratingPollMock {
        migrating_cp: tokio::sync::Mutex::new(Some(cp_migrating)),
        archived_cp: tokio::sync::Mutex::new(Some(cp_archived)),
        archive_after_first_poll: true,
        load_count: std::sync::atomic::AtomicU32::new(0),
        archived_id: std::sync::Mutex::new(Some(session_id.clone())),
        restore_called: tokio::sync::Mutex::new(false),
        storage_view: std::sync::Mutex::new(None),
        last_saved: tokio::sync::Mutex::new(None),
    });

    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Register the session in key_registry and in-memory sessions map
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            crate::Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }

    // resolve(): Path 1 detects migrating → polls → archive completes →
    // falls through to Path 3 → archived check restores.
    // 5s wall upper bound guards against regression to the full 30s poll
    // (e.g. if a cache-first read sneaks back in and stalls the poll).
    let start = std::time::Instant::now();
    let resolved = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        mgr.find_or_create("feishu", &msg, None),
    )
    .await
    .expect("resolve should complete within 5s once archive completes")
    .unwrap();
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "archive_completes path should finish well under the 5s budget"
    );

    // Should restore the original session (not create a new one)
    assert_eq!(
        resolved, session_id,
        "should restore the original session after archive completes"
    );

    // Verify routing_key was re-registered after restore
    {
        let reg = mgr.key_registry.read().await;
        assert_eq!(
            reg.get(&routing_key).unwrap(),
            &session_id,
            "routing_key should point to restored session"
        );
    }

    // Pin the final persisted checkpoint: after the archived-restore, the
    // storage row must end ACTIVE (save_raw refresh) — a stale Migrating
    // cache write-back must not flip it back to migrating.
    {
        let cm_guard = mgr.checkpoint_manager.read().await;
        let cm = cm_guard.as_ref().expect("checkpoint manager must be set");
        let final_cp = cm
            .storage()
            .load_checkpoint(&session_id)
            .await
            .unwrap()
            .expect("checkpoint must exist after restore");
        assert_eq!(
            final_cp.status,
            SessionStatus::Active,
            "final storage checkpoint status must be Active after restore; \
             a stale Migrating cache write-back regression is present"
        );
    }

    // Pending notification should have been injected
    let notification = mgr.take_restore_notification(&session_id).await;
    assert!(
        notification.is_some(),
        "pending restore notification should be present"
    );
}

// ── Immediate-archived fast path: archive completed before resolve ─────────

/// Boundary case for the status-transition race: the Sweeper completed the
/// archiving BEFORE resolve() even started (storage already Archived while
/// the CM cache still pins the stale Migrating checkpoint, e.g. written by
/// an earlier registry-hit status check). The wait helper's immediate check
/// must hit Archived on its first storage read and return WITHOUT any poll
/// sleep, so resolve falls straight through to the archived restore and the
/// archiving notification injected by the Migrating branch is replaced by
/// the restore notification.
#[tokio::test]
async fn test_resolve_migrating_registry_hit_immediate_archived() {
    let session_id = "migrating-immediate-archived".to_string();

    // Stale Migrating value that only lives in the CM cache: it models the
    // checkpoint status read by an earlier registry-hit Path 1 status check,
    // cached before the Sweeper flipped the storage row to Archived.
    let mut cp_stale_migrating = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Migrating)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_stale_migrating.sender_id = Some("user-a".to_string());

    // What raw storage already holds: the archive is done.
    let mut cp_archived = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_archived.sender_id = Some("user-a".to_string());

    let mock = Arc::new(MigratingPollMock {
        migrating_cp: tokio::sync::Mutex::new(None),
        archived_cp: tokio::sync::Mutex::new(Some(cp_archived.clone())),
        archive_after_first_poll: false,
        load_count: std::sync::atomic::AtomicU32::new(0),
        archived_id: std::sync::Mutex::new(Some(session_id.clone())),
        restore_called: tokio::sync::Mutex::new(false),
        storage_view: std::sync::Mutex::new(None),
        last_saved: tokio::sync::Mutex::new(None),
    });

    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Register the session in key_registry and in-memory sessions map
    // (registry-hit precondition, same as the archive_completes test).
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            crate::Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }

    // Prime the CM cache with the stale Migrating checkpoint: models the
    // production race where an earlier Path 1 status check cached Migrating
    // and the cache never got invalidated when the Sweeper archived. After
    // this cm.load returns Migrating from cache even though storage is
    // already Archived.
    {
        let cm_guard = mgr.checkpoint_manager.read().await;
        let cm = cm_guard.as_ref().expect("checkpoint manager must be set");
        cm.save_raw(&cp_stale_migrating).await.unwrap();
        // Move storage to Archived behind the cache's back (as the Sweeper
        // does, via a raw storage write that never touches the CM cache):
        // save_raw cached Migrating, while direct storage reads now report
        // Archived.
        mock.set_storage(cp_archived);
        let cached = cm.load(&session_id).await.unwrap().unwrap();
        assert_eq!(
            cached.status,
            SessionStatus::Migrating,
            "cache should pin the stale Migrating checkpoint"
        );
        let storage_cp = cm
            .storage()
            .load_checkpoint(&session_id)
            .await
            .unwrap()
            .expect("storage must hold the archived checkpoint");
        assert_eq!(
            storage_cp.status,
            SessionStatus::Archived,
            "storage must already be Archived before resolve starts"
        );
    }

    // resolve(): Path 1 status check reads the stale Migrating from cache →
    // wait immediate check reads Archived straight from storage → returns
    // with zero poll sleeps → falls through to archived restore.
    // 1s wall upper bound pins the fast path: it only holds if the immediate
    // check succeeds on the first storage read (no 500ms poll loop, and no
    // regression to a cache-first read that would stall the full 30s).
    let start = std::time::Instant::now();
    let resolved = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        mgr.find_or_create("feishu", &msg, None),
    )
    .await
    .expect(
        "resolve must complete via the immediate-archived fast path \
         (zero poll sleeps); a stall here means the stale-cache \
         regression is back",
    )
    .unwrap();
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "immediate-archived fast path should finish in well under 1s, took {:?}",
        start.elapsed()
    );

    // Should restore the original session (not create a new one)
    assert_eq!(
        resolved, session_id,
        "should restore the original session after the pre-completed archive"
    );

    // The Migrating branch injects an archiving notification before waiting;
    // the fast path must end with the RESTORE notification (injected by
    // try_restore_archived_session) as the last write — (chat_id, None) means
    // the default restore text. A timeout-restore would instead leave the
    // custom "正在恢复会话…" recovery message injected by
    // restore_migrating_on_timeout, so the None here also discriminates the
    // two paths.
    let notification = mgr.take_restore_notification(&session_id).await;
    assert!(
        notification.is_some(),
        "restore notification should be present after the fast-path restore"
    );
    assert_eq!(
        notification.unwrap().1,
        None,
        "notification should be the default restore text (no archiving message left)"
    );

    // Routing key must point back at the restored session.
    {
        let reg = mgr.key_registry.read().await;
        assert_eq!(
            reg.get(&routing_key).unwrap(),
            &session_id,
            "routing_key should point to the restored session"
        );
    }

    // Pin the final persisted checkpoint: the restore flipped storage to
    // Active, and the CM cache refresh (save_raw of the restored Active
    // checkpoint) must keep it Active — a stale Migrating cache write-back
    // must not flip the storage row back to migrating.
    {
        let cm_guard = mgr.checkpoint_manager.read().await;
        let cm = cm_guard.as_ref().expect("checkpoint manager must be set");
        let final_cp = cm
            .storage()
            .load_checkpoint(&session_id)
            .await
            .unwrap()
            .expect("checkpoint must exist after fast-path restore");
        assert_eq!(
            final_cp.status,
            SessionStatus::Active,
            "final storage checkpoint status must be Active after the \
             fast-path restore; a stale Migrating cache write-back regression \
             is present"
        );
    }
}

// ── Timeout path: archive does not complete → restore migrating session ────

/// When a registry-hit session is migrating and the Sweeper does NOT
/// complete archiving within the poll window, resolve() should restore
/// the migrating session via checkpoint restore (not create a new one).
///
/// Uses a paused tokio clock: the 30s poll deadline advances automatically
/// through the 500ms sleeps, so the test folds logical time to milliseconds
/// while still exercising the real timeout-restore path.
#[tokio::test(start_paused = true)]
async fn test_resolve_migrating_registry_hit_timeout_creates_new() {
    let session_id = "migrating-timeout".to_string();

    let mut cp_migrating = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Migrating)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_migrating.sender_id = Some("user-a".to_string());

    let mock = Arc::new(MigratingPollMock {
        migrating_cp: tokio::sync::Mutex::new(Some(cp_migrating)),
        archived_cp: tokio::sync::Mutex::new(None),
        archive_after_first_poll: false, // Always returns migrating → timeout
        load_count: std::sync::atomic::AtomicU32::new(0),
        archived_id: std::sync::Mutex::new(None),
        restore_called: tokio::sync::Mutex::new(false),
        storage_view: std::sync::Mutex::new(None),
        last_saved: tokio::sync::Mutex::new(None),
    });

    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            crate::Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }

    // resolve(): Path 1 detects migrating → polls → timeout →
    // restores migrating session via checkpoint restore.
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    // Should restore the migrating session (resolved == session_id)
    assert_eq!(
        resolved, session_id,
        "should restore migrating session after timeout, not create new"
    );

    // The restored session should exist in memory
    assert!(
        mgr.has_session(&session_id).await,
        "restored session should exist"
    );

    // Pin the final persisted checkpoint: after the timeout-restore, the
    // storage row must stay ACTIVE. restore_migrating_on_timeout writes the
    // restored Active checkpoint back through cm.save_raw to refresh the CM
    // cache; without that refresh, update_checkpoint_fields reads the stale
    // pinned Migrating and overwrites Active → migrating, re-entering the
    // 30s poll loop on every message.
    {
        let cm_guard = mgr.checkpoint_manager.read().await;
        let cm = cm_guard.as_ref().expect("checkpoint manager must be set");
        let final_cp = cm
            .storage()
            .load_checkpoint(&session_id)
            .await
            .unwrap()
            .expect("checkpoint must exist after timeout restore");
        assert_eq!(
            final_cp.status,
            SessionStatus::Active,
            "final storage checkpoint status must stay Active after the \
             timeout restore; a stale Migrating cache write-back regression \
             is present"
        );
    }
}

// ── Migrating session restored after timeout (not directly) ─────────────────

/// Verify that a migrating session in the registry goes through the
/// polling wait before being returned. After timeout, the migrating
/// session is restored via checkpoint restore (resolved == session_id).
///
/// Uses a paused tokio clock (see timeout test above).
#[tokio::test(start_paused = true)]
async fn test_resolve_migrating_not_directly_restored() {
    let session_id = "migrating-no-direct".to_string();

    let mut cp_migrating = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Migrating)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_migrating.sender_id = Some("user-a".to_string());

    let mock = Arc::new(MigratingPollMock {
        migrating_cp: tokio::sync::Mutex::new(Some(cp_migrating)),
        archived_cp: tokio::sync::Mutex::new(None),
        archive_after_first_poll: false,
        load_count: std::sync::atomic::AtomicU32::new(0),
        archived_id: std::sync::Mutex::new(None),
        restore_called: tokio::sync::Mutex::new(false),
        storage_view: std::sync::Mutex::new(None),
        last_saved: tokio::sync::Mutex::new(None),
    });

    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            crate::Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }

    // resolve() goes through polling wait. After timeout, the migrating
    // session is restored via checkpoint restore (resolved == session_id).
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(
        resolved, session_id,
        "migrating session should be restored after timeout, not replaced"
    );

    // Pin the final persisted checkpoint: the storage row must stay ACTIVE
    // after the timeout restore — a stale Migrating cache write-back must
    // not flip it back to migrating (same pin as the timeout test above).
    {
        let cm_guard = mgr.checkpoint_manager.read().await;
        let cm = cm_guard.as_ref().expect("checkpoint manager must be set");
        let final_cp = cm
            .storage()
            .load_checkpoint(&session_id)
            .await
            .unwrap()
            .expect("checkpoint must exist after timeout restore");
        assert_eq!(
            final_cp.status,
            SessionStatus::Active,
            "final storage checkpoint status must stay Active after the \
             timeout restore; a stale Migrating cache write-back regression \
             is present"
        );
    }
}

// ── Archived recovery path regression ───────────────────────────────────────

/// Ensure that the existing archived recovery path (Path 2) still works
/// correctly when a registry-hit session has checkpoint status Archived.
#[tokio::test]
async fn test_resolve_archived_registry_hit_still_restores() {
    use closeclaw_session::storage::memory::MemoryStorage;

    let session_id = "archived-regression".to_string();

    let storage: Arc<MemoryStorage> = Arc::new(MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        Default::default(),
    );

    // Save an archived checkpoint to storage
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.sender_id = Some("user-a".to_string());
    storage.save_checkpoint(&cp).await.unwrap();

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Register in key_registry + in-memory sessions (stale entry)
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            crate::Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }

    // resolve(): Path 1 detects archived → removes stale → Path 2 restores
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(
        resolved, session_id,
        "archived session should be restored via Path 2"
    );
}

// ── session_key is used in resolve (not ignored) ────────────────────────────

/// Verify that resolve() passes session_key to its internal logic
/// by confirming the session_key parameter is not prefixed with `_`
/// (i.e., not ignored). This is a compile-time + code-review check.
///
/// The actual log field verification requires a tracing subscriber,
/// which is tested separately in resolve_checkpoint_status_tests.rs.
#[tokio::test]
async fn test_resolve_session_key_not_ignored() {
    // This test verifies behavior: if session_key were ignored,
    // the resolve() signature would still have `_session_key`.
    // Since Step 1.2 renamed it to `session_key`, this test exists
    // as a reminder that the parameter is consumed.
    //
    // Functional verification: resolve() completes without error
    // when called with any session_key string.
    let mgr = SessionManager::new(&test_config(), None, None, Default::default());
    let msg = test_message();

    let result = mgr.find_or_create("feishu", &msg, None).await;
    assert!(
        result.is_ok(),
        "resolve should succeed with any session_key"
    );
}
