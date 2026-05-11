use super::*;
use crate::gateway::{GatewayConfig, Message};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{AgentRole, PersistenceError, SessionCheckpoint};
use async_trait::async_trait;
use std::io::Write;
use tempfile::TempDir;
use tokio::sync::Mutex;

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        dm_scope: DmScope::PerChannelPeer,
    }
}

fn test_message() -> Message {
    Message {
        id: "msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
    }
}

#[tokio::test]
async fn test_find_or_create_existing_session() {
    let mgr = SessionManager::new(&test_config(), None, None, BootstrapMode::Full);
    let msg = test_message();
    let session_id = mgr.compute_session_key("feishu", &msg, None);
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            Session {
                id: session_id.clone(),
                agent_id: "agent-b".to_string(),
                channel: "feishu".to_string(),
                created_at: chrono::Utc::now().timestamp(),
            },
        );
    }

    // find_or_create should return the existing session (read-lock fast path)
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, session_id);
}

#[tokio::test]
async fn test_find_or_create_new_user_creates_session() {
    let mgr = SessionManager::new(&test_config(), None, None, BootstrapMode::Full);
    let msg = test_message();

    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let sessions = mgr.sessions.read().await;
    assert!(sessions.contains_key(&result));
    let session = sessions.get(&result).unwrap();
    assert_eq!(session.agent_id, "agent-b");
    assert_eq!(session.channel, "feishu");
}

#[tokio::test]
async fn test_archived_session_restoration() {
    use crate::session::persistence::SessionCheckpoint;
    use std::sync::Arc;

    // Build a mock storage that returns an Archived checkpoint
    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: Mutex::new(Some(
            SessionCheckpoint::new("feishu:user-a:agent-b".to_string())
                .with_status(SessionStatus::Archived)
                .with_chat_id("agent-b".to_string()),
        )),
        restore_called: Mutex::new(false),
    });

    let config = test_config();
    let mut mgr = SessionManager::new(
        &config,
        Some(mock_storage.clone()),
        None,
        BootstrapMode::Full,
    );
    let msg = test_message();

    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "feishu:user-a:agent-b");

    // Verify restore was called
    let called = *mock_storage.restore_called.lock().await;
    assert!(called, "restore_checkpoint should have been called");
}

// ── Bootstrap injection tests ────────────────────────────────────────────────

/// Helper: create a temp workspace with the given file names and content.
fn make_temp_workspace(files: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    for (name, content) in files {
        let path = tmp.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }
    tmp
}

/// Test 1: workspace_dir + BootstrapMode::Full → system prompt contains all 7 files.
#[tokio::test]
async fn test_bootstrap_full_injects_all_files() {
    let tmp = make_temp_workspace(&[
        ("AGENTS.md", "agents content"),
        ("SOUL.md", "soul content"),
        ("IDENTITY.md", "identity content"),
        ("USER.md", "user content"),
        ("TOOLS.md", "tools content"),
        ("BOOTSTRAP.md", "bootstrap content"),
        ("MEMORY.md", "memory content"),
    ]);
    let workspace_dir = Some(tmp.path().to_path_buf());
    let mgr = SessionManager::new(
        &test_config(),
        None,
        workspace_dir.clone(),
        BootstrapMode::Full,
    );
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let prompt = conv.system_prompt().expect("expected system prompt");

    // All 7 files must appear in the system prompt
    assert!(prompt.contains("## AGENTS.md"), "missing AGENTS.md");
    assert!(prompt.contains("## SOUL.md"), "missing SOUL.md");
    assert!(prompt.contains("## IDENTITY.md"), "missing IDENTITY.md");
    assert!(prompt.contains("## USER.md"), "missing USER.md");
    assert!(prompt.contains("## TOOLS.md"), "missing TOOLS.md");
    assert!(prompt.contains("## BOOTSTRAP.md"), "missing BOOTSTRAP.md");
    assert!(prompt.contains("## MEMORY.md"), "missing MEMORY.md");
    // File contents must be present too
    assert!(prompt.contains("agents content"));
    assert!(prompt.contains("memory content"));
}

/// Test 2: workspace_dir + BootstrapMode::Minimal → system prompt contains only 5 minimal files.
#[tokio::test]
async fn test_bootstrap_minimal_injects_five_files() {
    let tmp = make_temp_workspace(&[
        ("AGENTS.md", "agents content"),
        ("SOUL.md", "soul content"),
        ("IDENTITY.md", "identity content"),
        ("USER.md", "user content"),
        ("TOOLS.md", "tools content"),
        ("BOOTSTRAP.md", "bootstrap content"),
        ("MEMORY.md", "memory content"),
    ]);
    let workspace_dir = Some(tmp.path().to_path_buf());
    let mgr = SessionManager::new(&test_config(), None, workspace_dir, BootstrapMode::Minimal);
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let prompt = conv.system_prompt().expect("expected system prompt");

    // Minimal mode: only 5 files, no BOOTSTRAP.md or MEMORY.md
    assert!(prompt.contains("## AGENTS.md"), "missing AGENTS.md");
    assert!(prompt.contains("## SOUL.md"), "missing SOUL.md");
    assert!(prompt.contains("## IDENTITY.md"), "missing IDENTITY.md");
    assert!(prompt.contains("## USER.md"), "missing USER.md");
    assert!(prompt.contains("## TOOLS.md"), "missing TOOLS.md");
    assert!(
        !prompt.contains("## BOOTSTRAP.md"),
        "BOOTSTRAP.md should not be in Minimal mode"
    );
    assert!(
        !prompt.contains("## MEMORY.md"),
        "MEMORY.md should not be in Minimal mode"
    );
}

/// Test 3: No workspace_dir → ConversationSession has no system prompt.
#[tokio::test]
async fn test_no_workspace_dir_no_system_prompt() {
    let mgr = SessionManager::new(&test_config(), None, None, BootstrapMode::Full);
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    assert!(
        conv.system_prompt().is_none(),
        "expected no system prompt when workspace_dir is None"
    );
}

/// Test 4: Partial bootstrap files → system prompt contains only existing files.
#[tokio::test]
async fn test_partial_bootstrap_files() {
    let tmp = make_temp_workspace(&[
        ("AGENTS.md", "agents only"),
        ("SOUL.md", "soul only"),
        // IDENTITY.md, USER.md, TOOLS.md, BOOTSTRAP.md, MEMORY.md are missing
    ]);
    let mgr = SessionManager::new(
        &test_config(),
        None,
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
    );
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let prompt = conv.system_prompt().expect("expected system prompt");

    assert!(prompt.contains("## AGENTS.md"), "missing AGENTS.md");
    assert!(prompt.contains("## SOUL.md"), "missing SOUL.md");
    assert!(!prompt.contains("## IDENTITY.md"), "unexpected IDENTITY.md");
    assert!(
        !prompt.contains("## BOOTSTRAP.md"),
        "unexpected BOOTSTRAP.md"
    );
    assert!(prompt.contains("agents only"));
    assert!(prompt.contains("soul only"));
}

#[tokio::test]
async fn test_find_or_create_no_storage() {
    // When storage is None, archived restoration returns false,
    // and find_or_create should still successfully create a new session.
    let mgr = SessionManager::new(&test_config(), None, None, BootstrapMode::Full);
    let msg = test_message();

    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "feishu:user-a:agent-b");

    let sessions = mgr.sessions.read().await;
    assert!(sessions.contains_key(&result));
    let session = sessions.get(&result).unwrap();
    assert_eq!(session.agent_id, "agent-b");
    assert_eq!(session.channel, "feishu");
}

// Mock persistence service for tests
struct MockPersistService {
    archived_checkpoint: Mutex<Option<crate::session::persistence::SessionCheckpoint>>,
    restore_called: Mutex<bool>,
}

#[async_trait::async_trait]
impl PersistenceService for MockPersistService {
    async fn save_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.archived_checkpoint.lock().await.take())
    }

    async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        *self.restore_called.lock().await = true;
        Ok(self.archived_checkpoint.lock().await.take())
    }

    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn purge_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn invalidate_session(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_idle_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: AgentRole,
        _idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: AgentRole,
        _purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}
