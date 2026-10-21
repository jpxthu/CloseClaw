//! Unit tests for CheckpointManager — cache + persistence coordination.

use crate::checkpoint_manager::CheckpointManager;
use crate::llm_session::SessionMessage;
use crate::persistence::{AgentRole, PersistenceError, PersistenceService, SessionCheckpoint};
use closeclaw_test_support::log_capture::capture_logs_async;
use std::sync::{Arc, Mutex};

/// In-memory storage for tests.
#[derive(Debug, Default)]
struct MemStorage {
    checkpoints: Mutex<Vec<SessionCheckpoint>>,
    save_count: Mutex<usize>,
    /// When set, every `save_checkpoint` fails with this message
    /// (error-path tests for the save family).
    fail_save_message: Option<String>,
}

impl MemStorage {
    fn add_checkpoint(&self, cp: SessionCheckpoint) {
        self.checkpoints.lock().unwrap().push(cp);
    }

    /// Storage whose `save_checkpoint` always fails with the given message.
    fn failing(message: &str) -> Self {
        Self {
            fail_save_message: Some(message.to_string()),
            ..Default::default()
        }
    }

    /// Number of `save_checkpoint` attempts (successes and failures).
    fn save_attempts(&self) -> usize {
        *self.save_count.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl PersistenceService for MemStorage {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        *self.save_count.lock().unwrap() += 1;
        if let Some(message) = &self.fail_save_message {
            return Err(PersistenceError::Sqlite(message.clone()));
        }
        self.checkpoints.lock().unwrap().push(cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self
            .checkpoints
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.session_id == id)
            .cloned())
    }
    async fn delete_checkpoint(&self, id: &str) -> Result<(), PersistenceError> {
        self.checkpoints
            .lock()
            .unwrap()
            .retain(|c| c.session_id != id);
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn archive_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.checkpoints
            .lock()
            .unwrap()
            .retain(|c| c.session_id != cp.session_id);
        Ok(())
    }
    async fn purge_checkpoint(&self, _id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn invalidate_session(&self, _id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

fn make_cp(id: &str) -> SessionCheckpoint {
    SessionCheckpoint::new(id.to_string())
}

// ── Normal path: save → load cache hit ──────────────────────────────────────

#[tokio::test]
async fn test_save_then_load_cache_hit() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    let cp = make_cp("s1");
    cm.save(cp).await.unwrap();

    // load should hit the cache — no storage load needed
    let loaded = cm.load("s1").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().session_id, "s1");
    // save() spawns async write, but cache is updated immediately.
    // Verify cache has the entry.
    assert!(cm.cached_session_ids().await.contains(&"s1".to_string()));
}

// ── Normal path: save → cache cleared → load from storage ───────────────────

#[tokio::test]
async fn test_save_then_clear_cache_then_load_from_storage() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    // Use save_now to ensure storage is written before we clear cache
    let cp = make_cp("s2");
    cm.save_now(cp).await.unwrap();
    cm.clear_cache().await;

    // Cache is empty → load must go to storage
    let loaded = cm.load("s2").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().session_id, "s2");
}

// ── Normal path: load from storage populates cache ──────────────────────────

#[tokio::test]
async fn test_load_populates_cache() {
    let mem = Arc::new(MemStorage::default());
    mem.add_checkpoint(make_cp("s3"));

    let cm = CheckpointManager::new(mem.clone());

    // First load: cache miss → storage hit → cache populated
    let loaded = cm.load("s3").await.unwrap();
    assert_eq!(loaded.unwrap().session_id, "s3");

    // Second load: cache hit (no storage call needed)
    // Verify by checking cached_session_ids
    let ids = cm.cached_session_ids().await;
    assert!(ids.contains(&"s3".to_string()));
}

// ── Error path: load nonexistent returns None ───────────────────────────────

#[tokio::test]
async fn test_load_nonexistent_returns_none() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    let loaded = cm.load("nonexistent").await.unwrap();
    assert!(loaded.is_none());
}

// ── Edge: save_direct does not inject identity ─────────────────────────────────

#[tokio::test]
async fn test_save_direct_preserves_original_fields() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new_with_identity(
        mem.clone(),
        "custom-agent".into(),
        AgentRole::SubAgent,
    );

    let mut cp = make_cp("s4");
    cp.agent_id = Some("original-agent".into());
    cp.role = Some(AgentRole::MainAgent);

    cm.save_direct(&cp).await.unwrap();

    let loaded = cm.load("s4").await.unwrap().unwrap();
    // save_direct does NOT inject identity
    assert_eq!(loaded.agent_id.as_deref(), Some("original-agent"));
    assert_eq!(loaded.role, Some(AgentRole::MainAgent));
}

// ── Edge: save with identity injection ──────────────────────────────────────

#[tokio::test]
async fn test_save_injects_identity() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new_with_identity(
        mem.clone(),
        "injected-agent".into(),
        AgentRole::SubAgent,
    );

    let mut cp = make_cp("s5");
    cp.agent_id = Some("old-agent".into());
    cp.role = Some(AgentRole::MainAgent);

    cm.save(cp).await.unwrap();

    let loaded = cm.load("s5").await.unwrap().unwrap();
    assert_eq!(loaded.agent_id.as_deref(), Some("injected-agent"));
    assert_eq!(loaded.role, Some(AgentRole::SubAgent));
}

// ── Error path: save_now propagates storage failure to the caller ───────────

#[tokio::test]
async fn test_save_now_storage_failure_propagates_error() {
    let mem = Arc::new(MemStorage::failing("disk full"));
    let cm = CheckpointManager::new(mem.clone());

    // save_now awaits the storage write, so the failure must reach the caller
    let err = cm.save_now(make_cp("fail-save-now")).await.unwrap_err();
    assert!(
        matches!(err, PersistenceError::Sqlite(ref m) if m == "disk full"),
        "save_now must propagate the storage error, got: {err:?}"
    );
}

// ── Error path: save swallows background failure (returns Ok + error log) ───

/// `save()` spawns the storage write in the background and swallows its
/// error (log-only). Pin both halves: the call still returns `Ok(())`, and
/// the failed background write is reported via an error log.
///
/// Paused tokio clock (`start_paused = true`): the 10 ms poll sleep
/// auto-advances instantly, so the wait is deterministic instead of
/// hugging the 1 s wall-clock deadline.
#[tokio::test(start_paused = true)]
#[serial_test::serial]
async fn test_save_storage_failure_swallows_error_and_logs() {
    let mem = Arc::new(MemStorage::failing("disk full"));
    let cm = CheckpointManager::new(mem.clone());

    // Everything the capture needs runs inside the scope: the call itself
    // plus a deadline-bounded wait until the spawned write has attempted
    // (and failed) the storage save — only then are the logs complete.
    let (_, logs) = capture_logs_async(
        || async {
            let result = cm.save(make_cp("fail-save")).await;
            assert!(
                result.is_ok(),
                "save() must swallow the storage failure and return Ok(()), got: {result:?}"
            );

            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
            while mem.save_attempts() == 0 {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "background save task did not run in time"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        },
        tracing::Level::ERROR,
    )
    .await;

    assert!(
        logs.contains("Failed to save checkpoint"),
        "expected the swallowed failure to be logged as error, got: {logs}"
    );
    assert!(
        logs.contains("disk full"),
        "expected the storage error message in the log, got: {logs}"
    );
}

// ── Edge: delete removes from cache and storage ─────────────────────────────

#[tokio::test]
async fn test_delete_removes_from_cache_and_storage() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    cm.save_now(make_cp("s6")).await.unwrap();
    assert!(cm.load("s6").await.unwrap().is_some());

    cm.delete("s6").await.unwrap();

    // Removed from cache
    assert!(!cm.cached_session_ids().await.contains(&"s6".to_string()));
    // Removed from storage
    let loaded = cm.load("s6").await.unwrap();
    assert!(loaded.is_none());
}

// ── Edge: archive removes from cache and deletes from storage ───────────────

#[tokio::test]
async fn test_archive_removes_from_cache_and_storage() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    let cp = make_cp("s7");
    cm.save_now(cp.clone()).await.unwrap();

    cm.archive(cp).await.unwrap();

    // Removed from cache
    assert!(!cm.cached_session_ids().await.contains(&"s7".to_string()));
    // Removed from storage (delete was called)
    let loaded = cm.load("s7").await.unwrap();
    assert!(loaded.is_none());
}

// ── Edge: clear_cache empties cache but storage intact ──────────────────────

#[tokio::test]
async fn test_clear_cache_preserves_storage() {
    let mem = Arc::new(MemStorage::default());
    mem.add_checkpoint(make_cp("s8"));

    let cm = CheckpointManager::new(mem.clone());
    // Pre-populate cache via load
    cm.load("s8").await.unwrap();
    assert!(cm.cached_session_ids().await.contains(&"s8".to_string()));

    cm.clear_cache().await;
    assert!(cm.cached_session_ids().await.is_empty());

    // Storage still has it
    let loaded = cm.load("s8").await.unwrap();
    assert!(loaded.is_some());
}

// ── Boundary: empty checkpoint save/load ────────────────────────────────────

#[tokio::test]
async fn test_empty_checkpoint_save_load() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    let cp = SessionCheckpoint::new("empty-session".into());
    cm.save_now(cp).await.unwrap();

    let loaded = cm.load("empty-session").await.unwrap().unwrap();
    assert_eq!(loaded.session_id, "empty-session");
    assert!(loaded.outbound_pending.is_empty());
    assert!(loaded.user_appends.is_empty());
}

// ── Boundary: large checkpoint save/load ────────────────────────────────────

#[tokio::test]
async fn test_large_checkpoint_save_load() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    let mut cp = make_cp("large-session");
    // Create a large transcript (~1000 messages)
    for i in 0..1000 {
        cp.pending_messages.push(SessionMessage {
            role: "user".into(),
            content_blocks: vec![closeclaw_common::ContentBlock::Text(format!("message-{i}"))],
            timestamp: chrono::Utc::now(),
        });
    }

    cm.save_now(cp).await.unwrap();

    let loaded = cm.load("large-session").await.unwrap().unwrap();
    assert_eq!(loaded.pending_messages.len(), 1000);
}

// ── Boundary: load non-existent after clear cache → None ────────────────────

#[tokio::test]
async fn test_load_nonexistent_after_clear_cache() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    cm.clear_cache().await;
    let loaded = cm.load("ghost").await.unwrap();
    assert!(loaded.is_none());
}

// ── Boundary: repeated save overwrites cache ────────────────────────────────

#[tokio::test]
async fn test_repeated_save_overwrites_cache() {
    let mem = Arc::new(MemStorage::default());
    let cm = CheckpointManager::new(mem.clone());

    let mut cp1 = make_cp("s9");
    cp1.platform = Some("old-platform".into());
    cm.save_now(cp1).await.unwrap();

    let mut cp2 = make_cp("s9");
    cp2.platform = Some("new-platform".into());
    cm.save_now(cp2).await.unwrap();

    let loaded = cm.load("s9").await.unwrap().unwrap();
    assert_eq!(loaded.platform.as_deref(), Some("new-platform"));
}
