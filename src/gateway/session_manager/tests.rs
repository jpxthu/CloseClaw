use super::*;
use crate::gateway::{GatewayConfig, Message};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{AgentRole, PersistenceError, SessionCheckpoint};
use crate::system_prompt::{
    invalidate_all_sections, set_agent_prompt, set_custom_prompt, set_override_prompt,
};
use async_trait::async_trait;
use serial_test::serial;
use std::io::Write;
use tempfile::TempDir;
use tokio::sync::Mutex;

/// Clear all global prompt state and section cache.
/// Must be called before each test that exercises system prompt generation,
/// because the global SECTION_CACHE and priority prompts are shared across tests.
fn clear_global_prompt_state() {
    set_override_prompt(None);
    set_agent_prompt(None);
    set_custom_prompt(None);
    invalidate_all_sections();
}

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
    let mgr = SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
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
    let mgr = SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
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
    use crate::session::persistence::ReasoningLevel;
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
        ReasoningLevel::default(),
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
/// Uses #[serial] because build_from_workspace shares a global SECTION_CACHE
/// that caches RoleSection by name — parallel tests would get stale cached content.
#[tokio::test]
#[serial]
async fn test_bootstrap_full_injects_all_files() {
    clear_global_prompt_state();

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
        ReasoningLevel::default(),
    );
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let prompt = conv.system_prompt().expect("expected system prompt");

    // All 7 files' content must appear in the system prompt (no section headers anymore)
    assert!(prompt.contains("agents content"), "missing agents content");
    assert!(prompt.contains("soul content"), "missing soul content");
    assert!(
        prompt.contains("identity content"),
        "missing identity content"
    );
    assert!(prompt.contains("user content"), "missing user content");
    assert!(prompt.contains("tools content"), "missing tools content");
    assert!(
        prompt.contains("bootstrap content"),
        "missing bootstrap content"
    );
    assert!(prompt.contains("memory content"), "missing memory content");
}

/// Test 2: workspace_dir + BootstrapMode::Minimal → system prompt contains only 5 minimal files.
#[tokio::test]
#[serial]
async fn test_bootstrap_minimal_injects_five_files() {
    clear_global_prompt_state();

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
        workspace_dir,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    );
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let prompt = conv.system_prompt().expect("expected system prompt");

    // Minimal mode: 5 files content present, no BOOTSTRAP.md content
    assert!(prompt.contains("agents content"), "missing agents content");
    assert!(prompt.contains("soul content"), "missing soul content");
    assert!(
        prompt.contains("identity content"),
        "missing identity content"
    );
    assert!(prompt.contains("user content"), "missing user content");
    assert!(prompt.contains("tools content"), "missing tools content");
    assert!(
        !prompt.contains("bootstrap content"),
        "unexpected bootstrap content"
    );
    // Note: MEMORY.md is read directly by build_from_workspace, so it may appear
    // even in minimal mode when the workspace contains MEMORY.md.
}

/// Test 3: No workspace_dir → system prompt exists (contains ToolsSection) but
/// has no bootstrap file content.
#[tokio::test]
#[serial]
async fn test_no_workspace_dir_no_system_prompt() {
    clear_global_prompt_state();

    let mgr = SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    // No workspace: system prompt should exist (contains ToolsSection), not None
    let prompt = conv.system_prompt().expect("expected system prompt");
    // Should not contain bootstrap file content
    assert!(
        !prompt.contains("agents content"),
        "unexpected bootstrap content"
    );
}

/// Test 4: Partial bootstrap files → system prompt contains only existing files.
#[tokio::test]
#[serial]
async fn test_partial_bootstrap_files() {
    clear_global_prompt_state();

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
        ReasoningLevel::default(),
    );
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let prompt = conv.system_prompt().expect("expected system prompt");

    // Only partial files present - check content, not section headers
    assert!(prompt.contains("agents only"), "missing agents only");
    assert!(prompt.contains("soul only"), "missing soul only");
    assert!(
        !prompt.contains("identity content"),
        "unexpected identity content"
    );
    assert!(
        !prompt.contains("bootstrap content"),
        "unexpected bootstrap content"
    );
}

/// Test 5: ToolRegistry injection → ToolsSection contains actual tool names.
#[tokio::test]
#[serial]
async fn test_find_or_create_with_tool_registry() {
    use crate::skills::DiskSkillRegistry;
    use crate::tools::builtin::register_builtin_tools;
    use crate::tools::ToolRegistry;

    clear_global_prompt_state();

    let tmp = make_temp_workspace(&[("AGENTS.md", "agents content")]);
    let workspace_dir = Some(tmp.path().to_path_buf());
    let mgr = SessionManager::new(
        &test_config(),
        None,
        workspace_dir,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    );

    // Inject ToolRegistry with builtin tools
    let disk_reg = Arc::new(DiskSkillRegistry::new(vec![]));
    let tool_registry = Arc::new(ToolRegistry::new());
    register_builtin_tools(&tool_registry, disk_reg).await;
    mgr.set_tool_registry(tool_registry).await;

    let msg = test_message();
    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let prompt = conv.system_prompt().expect("expected system prompt");

    // ToolsSection should contain actual tool names
    std::fs::write("/tmp/tool_registry_prompt.txt", &prompt).unwrap();
    assert!(prompt.contains("Read"), "missing Read tool");
    assert!(prompt.contains("Write"), "missing Write tool");
    // Bootstrap content should also be present
    assert!(
        prompt.contains("agents content"),
        "missing bootstrap content"
    );
}

#[tokio::test]
async fn test_find_or_create_no_storage() {
    let mgr = SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let msg = test_message();
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "feishu:user-a:agent-b");
    let sessions = mgr.sessions.read().await;
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
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.archived_checkpoint.lock().await.take())
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        *self.restore_called.lock().await = true;
        Ok(self.archived_checkpoint.lock().await.take())
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn purge_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _: &str) -> Result<(), PersistenceError> {
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

// ── Workspace creation tests ─────────────────────────────────────────────────

#[tokio::test]
async fn test_find_or_create_creates_workspace_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace_dir = Some(tmp.path().to_path_buf());
    let mgr = SessionManager::new(
        &test_config(),
        None,
        workspace_dir.clone(),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let msg = test_message();
    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let expected_workspace = workspace_dir
        .unwrap()
        .join("workspaces")
        .join("agent-b")
        .join("user-a");
    assert!(expected_workspace.exists());
    assert!(expected_workspace.is_dir());

    // Idempotent: calling again should not fail
    let session_id2 = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(session_id, session_id2);
}

#[tokio::test]
async fn test_find_or_create_no_workspace_dir_skipped() {
    let mgr = SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let msg = test_message();
    assert!(mgr.find_or_create("feishu", &msg, None).await.is_ok());
}

#[tokio::test]
async fn test_find_or_create_workspace_invalid_ids() {
    // invalid agent_id
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = SessionManager::new(
        &test_config(),
        None,
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let mut msg = test_message();
    msg.to = "../etc".to_string();
    assert!(mgr.find_or_create("feishu", &msg, None).await.is_err());

    // invalid user_id
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = SessionManager::new(
        &test_config(),
        None,
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let mut msg = test_message();
    msg.from = r#"..\foo"#.to_string();
    assert!(mgr.find_or_create("feishu", &msg, None).await.is_err());
}
