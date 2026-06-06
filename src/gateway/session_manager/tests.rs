use super::*;
use crate::gateway::{GatewayConfig, Message};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::SessionCheckpoint;
use crate::system_prompt::invalidate_all_sections;
use serial_test::serial;
use std::io::Write;
use tempfile::TempDir;
use tokio::sync::Mutex;

/// Clear section cache. Must be called before tests that exercise system prompt
/// generation (global SECTION_CACHE is shared).
pub(super) fn clear_global_prompt_state() {
    invalidate_all_sections();
}

pub(crate) fn test_config() -> GatewayConfig {
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
        thread_id: None,
    }
}

#[tokio::test]
async fn test_find_or_create_existing_session() {
    let mgr = make_test_mgr(None);
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
                depth: 0,
            },
        );
    }
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, session_id);
}

#[tokio::test]
async fn test_find_or_create_new_user_creates_session() {
    let mgr = make_test_mgr(None);
    let msg = test_message();
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let sessions = mgr.sessions.read().await;
    let session = sessions.get(&result).unwrap();
    assert_eq!(session.agent_id, "agent-b");
    assert_eq!(session.channel, "feishu");
}

#[tokio::test]
async fn test_archived_session_restoration() {
    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: Mutex::new(Some(
            SessionCheckpoint::new("feishu:user-a:agent-b".to_string())
                .with_status(SessionStatus::Archived)
                .with_chat_id("agent-b".to_string()),
        )),
        restore_called: Mutex::new(false),
    });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock_storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );
    let msg = test_message();
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "feishu:user-a:agent-b");
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

/// Shorthand for `SessionManager::new` with test defaults (Full mode, no storage).
pub(super) fn make_test_mgr(workspace: Option<&std::path::Path>) -> SessionManager {
    SessionManager::new(
        &test_config(),
        None,
        workspace.map(std::path::PathBuf::from),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    )
}

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

#[tokio::test]
#[serial]
async fn test_no_workspace_dir_no_system_prompt() {
    clear_global_prompt_state();
    let mgr = make_test_mgr(None);
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

#[tokio::test]
#[serial]
async fn test_find_or_create_with_tool_registry() {
    use crate::agent::spawn::SpawnController;
    use crate::config::ConfigManager;
    use crate::permission::engine::engine_eval::PermissionEngine;
    use crate::permission::rules::RuleSetBuilder;
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
    let mgr = Arc::new(mgr);

    // Inject ToolRegistry with builtin tools
    let disk_reg = Arc::new(DiskSkillRegistry::new(vec![]));
    let tool_registry = Arc::new(ToolRegistry::new());
    let perm_engine = Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ));
    // SpawnController needs a ConfigManager; in this test we just need the
    // registration call to succeed — empty agent config is fine.
    let cfg_mgr = Arc::new(
        ConfigManager::new(tmp.path().to_path_buf())
            .expect("failed to create ConfigManager for test"),
    );
    let spawn_controller = Arc::new(SpawnController::new(Arc::clone(&cfg_mgr), Arc::clone(&mgr)));
    register_builtin_tools(
        &tool_registry,
        disk_reg,
        perm_engine,
        spawn_controller,
        Arc::clone(&mgr),
        Arc::clone(&cfg_mgr),
    )
    .await;
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
    let mgr = make_test_mgr(None);
    let msg = test_message();
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "feishu:user-a:agent-b");
}

use super::test_helpers::MockPersistService;

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
    let expected = workspace_dir
        .unwrap()
        .join("workspaces")
        .join("agent-b")
        .join("user-a");
    assert!(expected.exists() && expected.is_dir());
    let session_id2 = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(session_id, session_id2);
}

#[tokio::test]
async fn test_find_or_create_no_workspace_dir_skipped() {
    let mgr = make_test_mgr(None);
    let msg = test_message();
    assert!(mgr.find_or_create("feishu", &msg, None).await.is_ok());
}

#[tokio::test]
async fn test_find_or_create_workspace_invalid_ids() {
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

#[tokio::test]
async fn test_force_new_for_channel_creates_valid_session() {
    let mgr = make_test_mgr(None);
    let new_id = mgr.force_new_for_channel("feishu", "agent-test").await;
    // Should be non-empty and contain the channel
    assert!(!new_id.is_empty());
    assert!(new_id.starts_with("feishu"));
    // Session should exist in the sessions map
    assert!(mgr.has_session(&new_id).await);
    // Channel mapping should be correct
    assert_eq!(
        mgr.active_session_for_channel("feishu").await.as_deref(),
        Some(new_id.as_str())
    );
    // ConversationSession should also exist
    assert!(mgr.get_conversation_session(&new_id).await.is_some());
}

#[tokio::test]
async fn test_clear_pending_with_messages() {
    let mgr = make_test_mgr(None);
    let msg = test_message();
    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    // Push some pending messages
    use crate::session::persistence::PendingMessage;
    mgr.push_pending_message(&session_id, PendingMessage::new("p1".into(), "msg1".into()))
        .await
        .unwrap();
    mgr.push_pending_message(&session_id, PendingMessage::new("p2".into(), "msg2".into()))
        .await
        .unwrap();
    assert_eq!(
        mgr.get_conversation_session(&session_id)
            .await
            .unwrap()
            .read()
            .await
            .pending_count(),
        2
    );
    // Clear pending
    let count = {
        let cs = mgr.get_conversation_session(&session_id).await.unwrap();
        let mut cs = cs.write().await;
        cs.clear_pending()
    };
    assert_eq!(count, 2);
    assert_eq!(
        mgr.get_conversation_session(&session_id)
            .await
            .unwrap()
            .read()
            .await
            .pending_count(),
        0
    );
}

#[tokio::test]
async fn test_clear_pending_empty() {
    let mgr = make_test_mgr(None);
    let msg = test_message();
    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let count = {
        let cs = mgr.get_conversation_session(&session_id).await.unwrap();
        let mut cs = cs.write().await;
        cs.clear_pending()
    };
    assert_eq!(count, 0);
}
