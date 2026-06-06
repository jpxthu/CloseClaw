use super::*;
use crate::gateway::GatewayConfig;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{AgentRole, PersistenceError, SessionCheckpoint};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Mock persistence that records saved checkpoints and also supports
/// loading them (needed for the Bug 2 / Bug 3 round-trip tests).
struct Bug904MockStorage {
    saved_checkpoints: Mutex<Vec<SessionCheckpoint>>,
    loaded_checkpoints: std::sync::Mutex<std::collections::HashMap<String, SessionCheckpoint>>,
}

impl Bug904MockStorage {
    fn new() -> Self {
        Self {
            saved_checkpoints: Mutex::new(Vec::new()),
            loaded_checkpoints: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Seed a checkpoint that will be returned by `load_checkpoint`.
    fn with_loaded_checkpoint(self, cp: SessionCheckpoint) -> Self {
        self.loaded_checkpoints
            .lock()
            .unwrap()
            .insert(cp.session_id.clone(), cp);
        self
    }
}

#[async_trait]
impl crate::session::persistence::PersistenceService for Bug904MockStorage {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        self.saved_checkpoints.lock().await.push(checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.loaded_checkpoints.lock().unwrap().remove(session_id))
    }

    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
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

pub(super) fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        dm_scope: DmScope::PerChannelPeer,
    }
}

pub(super) fn test_message() -> crate::gateway::Message {
    crate::gateway::Message {
        id: "msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    }
}

pub(super) fn make_test_session(id: &str) -> Session {
    Session {
        id: id.to_string(),
        agent_id: "agent-b".to_string(),
        channel: "feishu".to_string(),
        created_at: chrono::Utc::now().timestamp(),
        depth: 0,
    }
}

// ===================================================================
// Bug #904: find_or_create writes thread_id to checkpoint (Bug 2)
// ===================================================================

#[tokio::test]
async fn test_find_or_create_writes_thread_id_to_checkpoint() {
    // Bug #904: find_or_create should write message.thread_id to the new checkpoint.
    let storage = Arc::new(Bug904MockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    let mut msg = test_message();
    msg.thread_id = Some("omt_from_inbound".to_string());

    let session_id = mgr
        .find_or_create("feishu", &msg, None)
        .await
        .expect("find_or_create should succeed");

    // The checkpoint saved by find_or_create should carry the thread_id.
    let saved = storage.saved_checkpoints.lock().await;
    let cp = saved
        .iter()
        .find(|c| c.session_id == session_id)
        .expect("should have saved a checkpoint for this session");
    assert_eq!(
        cp.thread_id.as_deref(),
        Some("omt_from_inbound"),
        "checkpoint thread_id should match message.thread_id"
    );
}

// ===================================================================
// Bug #904: flush_all preserves thread_id (Bug 3)
// ===================================================================

#[tokio::test]
async fn test_flush_all_preserves_existing_thread_id() {
    // Bug #904: flush_all should not discard an existing thread_id.
    let existing_cp = SessionCheckpoint::new("sess-flush-tid".to_string())
        .with_status(SessionStatus::Active)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string())
        .with_thread_id("omt_existing_thread".to_string());

    let storage = Arc::new(Bug904MockStorage::new().with_loaded_checkpoint(existing_cp));
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    // Insert a session so flush_all has something to iterate over.
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            "sess-flush-tid".to_string(),
            make_test_session("sess-flush-tid"),
        );
    }

    let result = mgr.flush_all().await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1);

    let saved = storage.saved_checkpoints.lock().await;
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved[0].thread_id.as_deref(),
        Some("omt_existing_thread"),
        "flush_all must preserve the existing thread_id"
    );
}

// ===================================================================
// Bug #920: fast path updates thread_id in checkpoint
// ===================================================================

#[tokio::test]
async fn test_active_session_fast_path_updates_thread_id() {
    // Bug #920: When a session already exists (active session fast path),
    // find_or_create should update the checkpoint's thread_id.
    let storage = Arc::new(Bug904MockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    // Step 1: Create session via slow path (first find_or_create)
    let mut msg1 = test_message();
    msg1.thread_id = Some("omt_initial".to_string());
    let session_id = mgr
        .find_or_create("feishu", &msg1, None)
        .await
        .expect("first find_or_create should succeed");

    // Verify slow path created checkpoint with initial thread_id
    let saved = storage.saved_checkpoints.lock().await;
    let cp_initial = saved
        .iter()
        .find(|c| c.session_id == session_id)
        .expect("should have saved checkpoint on slow path");
    assert_eq!(
        cp_initial.thread_id.as_deref(),
        Some("omt_initial"),
        "slow path should set thread_id"
    );
    drop(saved);

    // Step 2: Seed checkpoint for fast path update (mock removes on load)
    let reseed = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Active)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string())
        .with_thread_id("omt_initial".to_string());
    {
        storage
            .loaded_checkpoints
            .lock()
            .unwrap()
            .insert(session_id.clone(), reseed);
    }

    // Step 3: Second find_or_create hits active session fast path
    let mut msg2 = test_message();
    msg2.thread_id = Some("omt_updated".to_string());
    let session_id2 = mgr
        .find_or_create("feishu", &msg2, None)
        .await
        .expect("second find_or_create should succeed");
    assert_eq!(session_id, session_id2, "session_id should be the same");

    // Verify fast path updated checkpoint thread_id
    let saved = storage.saved_checkpoints.lock().await;
    let cp_updated = saved
        .iter()
        .rfind(|c| c.session_id == session_id)
        .expect("should have saved checkpoint on fast path");
    assert_eq!(
        cp_updated.thread_id.as_deref(),
        Some("omt_updated"),
        "active session fast path should update thread_id"
    );
}

#[tokio::test]
async fn test_channel_override_fast_path_updates_thread_id() {
    // Bug #920: When a channel override is active (channel-level fast path),
    // find_or_create should update the checkpoint's thread_id.
    let storage = Arc::new(Bug904MockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    // Step 1: Create session via slow path
    let mut msg1 = test_message();
    msg1.thread_id = None;
    let session_id = mgr
        .find_or_create("feishu", &msg1, None)
        .await
        .expect("first find_or_create should succeed");

    // Step 2: Register channel override so subsequent calls take fast path
    {
        let mut channel_map = mgr.channel_active_sessions.write().await;
        channel_map.insert("feishu".to_string(), session_id.clone());
    }

    // Step 3: Seed checkpoint for fast path update
    let reseed = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Active)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    {
        storage
            .loaded_checkpoints
            .lock()
            .unwrap()
            .insert(session_id.clone(), reseed);
    }

    // Step 4: find_or_create hits channel override fast path
    let mut msg2 = test_message();
    msg2.thread_id = Some("omt_channel".to_string());
    let session_id2 = mgr
        .find_or_create("feishu", &msg2, None)
        .await
        .expect("second find_or_create should succeed");
    assert_eq!(session_id, session_id2, "session_id should be the same");

    // Verify channel override fast path updated checkpoint thread_id
    let saved = storage.saved_checkpoints.lock().await;
    let cp_updated = saved
        .iter()
        .rfind(|c| c.session_id == session_id)
        .expect("should have saved checkpoint on fast path");
    assert_eq!(
        cp_updated.thread_id.as_deref(),
        Some("omt_channel"),
        "channel override fast path should update thread_id"
    );
}

#[tokio::test]
async fn test_fast_path_thread_id_none_overwrites_old_value() {
    // Bug #920: When thread_id is None, the fast path should overwrite
    // the existing thread_id value in the checkpoint.
    let storage = Arc::new(Bug904MockStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    // Step 1: Create session with a thread_id via slow path
    let mut msg1 = test_message();
    msg1.thread_id = Some("omt_old".to_string());
    let session_id = mgr
        .find_or_create("feishu", &msg1, None)
        .await
        .expect("first find_or_create should succeed");

    // Step 2: Seed checkpoint with old thread_id for fast path update
    let reseed = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Active)
        .with_platform("feishu".to_string())
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string())
        .with_thread_id("omt_old".to_string());
    {
        storage
            .loaded_checkpoints
            .lock()
            .unwrap()
            .insert(session_id.clone(), reseed);
    }

    // Step 3: Second find_or_create with thread_id = None (fast path)
    let mut msg2 = test_message();
    msg2.thread_id = None;
    let session_id2 = mgr
        .find_or_create("feishu", &msg2, None)
        .await
        .expect("second find_or_create should succeed");
    assert_eq!(session_id, session_id2, "session_id should be the same");

    // Verify thread_id was overwritten to None
    let saved = storage.saved_checkpoints.lock().await;
    let cp_updated = saved
        .iter()
        .rfind(|c| c.session_id == session_id)
        .expect("should have saved checkpoint on fast path");
    assert_eq!(
        cp_updated.thread_id, None,
        "fast path should overwrite thread_id with None"
    );
}
