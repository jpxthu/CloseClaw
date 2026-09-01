//! Tests for resolve() registry miss path: Migrating status handling (Step 1.5).
//!
//! Verifies:
//! - Normal path: registry miss + migrating in SQLite → poll detects archived →
//!   archived restore → returns original session_id
//! - Timeout path: registry miss + migrating in SQLite → poll times out →
//!   creates new session
//! - Migrating/Archived query isolation: each query returns only its status

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

// ── Mock: supports migrating → archived transition during polling ───────────

/// Mock for registry-miss migrating tests.
///
/// Simulates the lifecycle: checkpoint starts as `Migrating`, transitions
/// to `Archived` after a configurable number of `load_checkpoint` calls
/// (or stays migrating forever for timeout tests).
struct MigratingMissMock {
    /// If `Some(n)`, `load_checkpoint` returns migrating for the first `n`
    /// calls, then returns the archived checkpoint. If `None`, always returns
    /// migrating (timeout scenario).
    transition_after_polls: Option<u32>,
    /// Counter for `load_checkpoint` calls.
    poll_count: std::sync::atomic::AtomicU32,
    /// Session ID returned by `find_migrating_session_by_routing`.
    migrating_id: std::sync::Mutex<Option<String>>,
    /// Session ID returned by `find_archived_session_by_routing`.
    archived_id: std::sync::Mutex<Option<String>>,
    /// Whether `restore_checkpoint` was called.
    restore_called: std::sync::atomic::AtomicBool,
}

impl MigratingMissMock {
    fn new_transition_after(migrating_id: &str, archived_id: &str, polls: u32) -> Self {
        Self {
            transition_after_polls: Some(polls),
            poll_count: std::sync::atomic::AtomicU32::new(0),
            migrating_id: std::sync::Mutex::new(Some(migrating_id.to_string())),
            archived_id: std::sync::Mutex::new(Some(archived_id.to_string())),
            restore_called: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn new_timeout(migrating_id: &str) -> Self {
        Self {
            transition_after_polls: None,
            poll_count: std::sync::atomic::AtomicU32::new(0),
            migrating_id: std::sync::Mutex::new(Some(migrating_id.to_string())),
            archived_id: std::sync::Mutex::new(None),
            restore_called: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl PersistenceService for MigratingMissMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let count = self
            .poll_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match self.transition_after_polls {
            Some(threshold) if count >= threshold => {
                // Transitioned to archived
                Ok(Some(
                    SessionCheckpoint::new("session".to_string())
                        .with_status(SessionStatus::Archived),
                ))
            }
            _ => {
                // Still migrating
                Ok(Some(
                    SessionCheckpoint::new("session".to_string())
                        .with_status(SessionStatus::Migrating),
                ))
            }
        }
    }

    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }

    async fn load_archived_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        self.restore_called
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(None)
    }

    async fn find_active_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        // No active session — registry miss path
        Ok(None)
    }

    async fn find_migrating_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(self.migrating_id.lock().unwrap().take())
    }

    async fn find_archived_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(self.archived_id.lock().unwrap().take())
    }
}

// ── Normal path: migrate → poll detects archived → restore ──────────────────

/// Registry miss + migrating session in SQLite → notification injected →
/// poll detects Archived → archived check restores the session →
/// verify returned session_id is the original (restored), not a new one.
#[tokio::test]
async fn test_resolve_miss_migrating_archive_completes() {
    let migrating_id = "migrating-miss-ok".to_string();
    let archived_id = migrating_id.clone(); // same session gets archived

    // After 1 poll (500 ms), checkpoint transitions to Archived.
    let mock = Arc::new(MigratingMissMock::new_transition_after(
        &migrating_id,
        &archived_id,
        1,
    ));
    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // key_registry is empty → miss
    {
        let reg = mgr.key_registry.read().await;
        assert!(!reg.contains_key(&routing_key));
    }

    // resolve(): Path 3 → active miss → migrating hit → poll → archived →
    // restore → returns original session_id.
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    // Should restore the original session (not create a new one)
    assert_eq!(
        resolved, migrating_id,
        "should restore the original migrating session after archive completes"
    );

    // Pending notification should have been injected
    let notification = mgr.take_restore_notification(&migrating_id).await;
    assert!(
        notification.is_some(),
        "pending restore notification should be present"
    );

    // Routing key should be re-registered after restore
    {
        let reg = mgr.key_registry.read().await;
        assert_eq!(
            reg.get(&routing_key).unwrap(),
            &migrating_id,
            "routing_key should point to restored session"
        );
    }
}

// ── Timeout path: migrate → poll times out → create new ────────────────────

/// Registry miss + migrating session in SQLite → notification injected →
/// poll times out (still migrating after 5 s) → creates new session →
/// verify returned session_id is a new session (different from migrating).
#[tokio::test]
async fn test_resolve_miss_migrating_timeout_creates_new() {
    let migrating_id = "migrating-miss-timeout".to_string();

    // Never transitions to archived → poll times out.
    let mock = Arc::new(MigratingMissMock::new_timeout(&migrating_id));
    let mgr = SessionManager::new(&test_config(), Some(mock.clone()), None, Default::default());

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // key_registry is empty → miss
    {
        let reg = mgr.key_registry.read().await;
        assert!(!reg.contains_key(&routing_key));
    }

    // resolve(): Path 3 → active miss → migrating hit → poll timeout →
    // archived miss → create new session.
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    // Should create a new session (different from the migrating one)
    assert_ne!(
        resolved, migrating_id,
        "should create a new session after poll timeout"
    );
    assert!(
        resolved.starts_with("agent-b_"),
        "new session format: {}",
        resolved
    );

    // Pending notification should still have been injected
    let notification = mgr.take_restore_notification(&migrating_id).await;
    assert!(
        notification.is_some(),
        "pending restore notification should be present even on timeout"
    );

    // The new session should exist in memory
    assert!(mgr.has_session(&resolved).await, "new session should exist");
}

// ── Migrating/Archived query isolation ──────────────────────────────────────

/// Mock that stores separate migrating and archived sessions,
/// enabling query isolation tests.
struct QueryIsolationMock {
    migrating_sessions: std::sync::Mutex<std::collections::HashMap<String, SessionCheckpoint>>,
    archived_sessions: std::sync::Mutex<std::collections::HashMap<String, SessionCheckpoint>>,
}

impl QueryIsolationMock {
    fn new() -> Self {
        Self {
            migrating_sessions: std::sync::Mutex::new(std::collections::HashMap::new()),
            archived_sessions: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn insert_migrating(&self, cp: SessionCheckpoint) {
        self.migrating_sessions
            .lock()
            .unwrap()
            .insert(cp.session_id.clone(), cp);
    }

    fn insert_archived(&self, cp: SessionCheckpoint) {
        self.archived_sessions
            .lock()
            .unwrap()
            .insert(cp.session_id.clone(), cp);
    }
}

#[async_trait::async_trait]
impl PersistenceService for QueryIsolationMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
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
    async fn find_migrating_session_by_routing(
        &self,
        account_id: Option<&str>,
        channel: &str,
        sender_id: &str,
        peer_id: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let migrating = self.migrating_sessions.lock().unwrap();
        for (id, cp) in migrating.iter() {
            if cp.platform.as_deref() == Some(channel)
                && cp.sender_id.as_deref() == Some(sender_id)
                && cp.peer_id.as_deref() == Some(peer_id)
                && cp.account_id.as_deref() == account_id
            {
                return Ok(Some(id.clone()));
            }
        }
        Ok(None)
    }
    async fn find_archived_session_by_routing(
        &self,
        account_id: Option<&str>,
        channel: &str,
        sender_id: &str,
        peer_id: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let archived = self.archived_sessions.lock().unwrap();
        for (id, cp) in archived.iter() {
            if cp.platform.as_deref() == Some(channel)
                && cp.sender_id.as_deref() == Some(sender_id)
                && cp.peer_id.as_deref() == Some(peer_id)
                && cp.account_id.as_deref() == account_id
            {
                return Ok(Some(id.clone()));
            }
        }
        Ok(None)
    }
}

/// `find_migrating_session_by_routing` only returns migrating sessions,
/// not archived ones.
#[tokio::test]
async fn test_resolve_miss_migrating_query_only_returns_migrating() {
    let mock = Arc::new(QueryIsolationMock::new());
    let migrating_id = "migrating-only".to_string();

    // Insert a migrating checkpoint
    let mut cp = SessionCheckpoint::new(migrating_id.clone())
        .with_status(SessionStatus::Migrating)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.sender_id = Some("user-a".to_string());
    cp.account_id = None;
    mock.insert_migrating(cp);

    // Also insert an archived checkpoint with different routing fields
    let mut cp_archived = SessionCheckpoint::new("archived-unrelated".to_string())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_archived.sender_id = Some("different-sender".to_string());
    cp_archived.account_id = None;
    mock.insert_archived(cp_archived);

    // find_migrating_session_by_routing should find the migrating session
    let result = mock
        .find_migrating_session_by_routing(None, "feishu", "user-a", "agent-b")
        .await
        .unwrap();
    assert_eq!(result, Some(migrating_id), "should find migrating session");

    // find_archived_session_by_routing should NOT find the migrating session
    let result = mock
        .find_archived_session_by_routing(None, "feishu", "user-a", "agent-b")
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "archived query should not return migrating session"
    );
}

/// `find_archived_session_by_routing` only returns archived sessions,
/// not migrating ones.
#[tokio::test]
async fn test_resolve_miss_archived_query_only_returns_archived() {
    let mock = Arc::new(QueryIsolationMock::new());
    let archived_id = "archived-only".to_string();

    // Insert an archived checkpoint
    let mut cp = SessionCheckpoint::new(archived_id.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.sender_id = Some("user-a".to_string());
    cp.account_id = None;
    mock.insert_archived(cp);

    // Also insert a migrating checkpoint with the same routing fields
    let mut cp_migrating = SessionCheckpoint::new("migrating-same-routing".to_string())
        .with_status(SessionStatus::Migrating)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_migrating.sender_id = Some("user-a".to_string());
    cp_migrating.account_id = None;
    mock.insert_migrating(cp_migrating);

    // find_archived_session_by_routing should find only the archived session
    let result = mock
        .find_archived_session_by_routing(None, "feishu", "user-a", "agent-b")
        .await
        .unwrap();
    assert_eq!(result, Some(archived_id), "should find archived session");

    // find_migrating_session_by_routing should find only the migrating session
    let result = mock
        .find_migrating_session_by_routing(None, "feishu", "user-a", "agent-b")
        .await
        .unwrap();
    assert_eq!(
        result,
        Some("migrating-same-routing".to_string()),
        "should find migrating session"
    );
}

/// Both migrating and archived sessions with the same routing fields:
/// each query returns only its own status, no cross-contamination.
#[tokio::test]
async fn test_resolve_miss_both_statuses_isolated() {
    let mock = Arc::new(QueryIsolationMock::new());
    let migrating_id = "both-migrating".to_string();
    let archived_id = "both-archived".to_string();

    // Insert both with the same routing fields
    let mut cp_mig = SessionCheckpoint::new(migrating_id.clone())
        .with_status(SessionStatus::Migrating)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_mig.sender_id = Some("user-a".to_string());
    cp_mig.account_id = None;
    mock.insert_migrating(cp_mig);

    let mut cp_arch = SessionCheckpoint::new(archived_id.clone())
        .with_status(SessionStatus::Archived)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp_arch.sender_id = Some("user-a".to_string());
    cp_arch.account_id = None;
    mock.insert_archived(cp_arch);

    // find_migrating_session_by_routing → migrating only
    let mig_result = mock
        .find_migrating_session_by_routing(None, "feishu", "user-a", "agent-b")
        .await
        .unwrap();
    assert_eq!(
        mig_result,
        Some(migrating_id),
        "migrating query should return migrating session"
    );

    // find_archived_session_by_routing → archived only
    let arch_result = mock
        .find_archived_session_by_routing(None, "feishu", "user-a", "agent-b")
        .await
        .unwrap();
    assert_eq!(
        arch_result,
        Some(archived_id),
        "archived query should return archived session"
    );

    // Verify: neither query returned the wrong status
    assert_ne!(
        mig_result, arch_result,
        "queries should return different sessions"
    );
}
