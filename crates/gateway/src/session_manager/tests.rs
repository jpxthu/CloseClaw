use super::*;
use crate::{GatewayConfig, Message};
use closeclaw_common::system_prompt::invalidate_all_sections;
use closeclaw_config::manager::ConfigSnapshot;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::SessionCheckpoint;
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
        ..Default::default()
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
    // First call: creates a new session via resolve Path 3
    let id1 = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert!(!id1.is_empty());
    // Second call: resolve finds the key in registry, returns same session
    let id2 = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(id1, id2, "same key should resolve to same session");
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
            SessionCheckpoint::new("test_sid".to_string())
                .with_status(SessionStatus::Archived)
                .with_peer_id("agent-b".to_string()),
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
    // Populate key_registry so resolve can look up the session.
    // resolve() strips timestamps before registry lookup — insert routing_key.
    let session_key = mgr.compute_session_key("feishu", &msg, None, msg.timestamp);
    let routing_key = SessionManager::strip_timestamp_from_session_key(&session_key);
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.to_string(), "test_sid".to_string());
    }
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "test_sid");
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
    use closeclaw_common::agent_lookup::spawn::SpawnController;
    use closeclaw_common::skill_registry::DiskSkillRegistry;
    use closeclaw_common::tool_registry::builtin::{register_builtin_tools, BuiltinToolContext};
    use closeclaw_common::tool_registry::ToolRegistry;
    use closeclaw_config::ConfigManager;
    use closeclaw_permission::engine::engine_eval::PermissionEngine;
    use closeclaw_permission::rules::RuleSetBuilder;

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
    let spawn_controller = Arc::new(SpawnController::new(
        Arc::clone(&cfg_mgr),
        Arc::clone(&mgr),
        perm_engine.clone(),
    ));
    let agent_registry = Arc::new(closeclaw_common::agent_lookup::registry::AgentRegistry::new());
    let builtin_ctx = Arc::new(BuiltinToolContext {
        config_manager: Arc::clone(&cfg_mgr),
        agent_registry,
        disk_registry: disk_reg,
        permission_engine: perm_engine,
        spawn_controller,
        session_manager: Arc::clone(&mgr),
    });
    register_builtin_tools(&tool_registry, builtin_ctx).await;
    mgr.set_tool_registry(tool_registry).await;

    let msg = test_message();
    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let prompt = conv.system_prompt().expect("expected system prompt");

    // ToolsSection should contain actual tool names
    // (debug write removed — use tempfile if needed)
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
    // Session ID should match the new format: {agent_id}_{ts}_{hex}
    assert!(result.starts_with("agent-b_"), "bad format: {}", result);
    let parts: Vec<&str> = result.rsplitn(2, '_').collect();
    assert_eq!(parts.len(), 2, "bad format: {}", result);
    assert_eq!(parts[0].len(), 8, "hex part wrong: {}", parts[0]);
}

use super::test_helpers::MockPersistService;

// ── Workspace creation tests ─────────────────────────────────────────────────

// =====================================================================
// notify_config_changed tests (Step 1.5)
// =====================================================================

/// notify_config_changed with no active sessions should not panic.
#[tokio::test]
async fn test_notify_config_changed_no_sessions() {
    let mgr = make_test_mgr(None);
    let snapshot: ConfigSnapshot = ConfigSnapshot::default();
    // No sessions created — should complete without error
    mgr.notify_config_changed(ConfigSection::Models, snapshot)
        .await;
}

/// notify_config_changed iterates over all active sessions and rebuilds
/// their system prompts without error.
#[tokio::test]
async fn test_notify_config_changed_iterates_active_sessions() {
    let mgr = make_test_mgr(None);
    let msg = test_message();

    // Create two sessions
    let id1 = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let mut msg2 = test_message();
    msg2.from = "user-c".to_string();
    msg2.to = "agent-d".to_string();
    let id2 = mgr.find_or_create("feishu", &msg2, None).await.unwrap();

    assert_ne!(id1, id2);

    // notify_config_changed should visit both sessions
    let snapshot: ConfigSnapshot = ConfigSnapshot::default();
    mgr.notify_config_changed(ConfigSection::Channels, snapshot)
        .await;

    // Both sessions should still exist and have conversation sessions
    assert!(mgr.has_session(&id1).await);
    assert!(mgr.has_session(&id2).await);
    assert!(mgr.get_conversation_session(&id1).await.is_some());
    assert!(mgr.get_conversation_session(&id2).await.is_some());
}

/// notify_config_changed with different sections does not interfere
/// with session state.
#[tokio::test]
async fn test_notify_config_changed_multiple_sections() {
    let mgr = make_test_mgr(None);
    let msg = test_message();
    let id = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    for section in [
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ] {
        mgr.notify_config_changed(section, ConfigSnapshot::default())
            .await;
    }

    // Session should still be intact
    assert!(mgr.has_session(&id).await);
    let cs = mgr.get_conversation_session(&id).await.unwrap();
    let cs = cs.read().await;
    assert!(cs.system_prompt().is_some(), "system prompt should exist");
}

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
    // New format: {agent_id}_{ts}_{hex}
    assert!(new_id.starts_with("agent-test_"), "bad format: {}", new_id);
    let parts: Vec<&str> = new_id.rsplitn(2, '_').collect();
    assert_eq!(parts.len(), 2, "bad format: {}", new_id);
    assert_eq!(parts[0].len(), 8, "hex part wrong: {}", parts[0]);
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
    use closeclaw_session::persistence::PendingMessage;
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

// =====================================================================
// Step 1.5 — swap_config_snapshot tests
// =====================================================================

/// swap_config_snapshot replaces the stored snapshot and subsequent reads
/// return the new value.
#[tokio::test]
async fn test_swap_config_snapshot() {
    use std::collections::HashMap;

    let mgr = make_test_mgr(None);

    // Initially no snapshot is stored
    assert!(
        mgr.get_config_snapshot().await.is_none(),
        "no snapshot should be stored initially"
    );

    // Create and swap in a snapshot
    let mut map = HashMap::new();
    map.insert(ConfigSection::System, serde_json::json!({"version": "5.0"}));
    let snapshot: ConfigSnapshot = ConfigSnapshot::new(map);
    mgr.swap_config_snapshot(snapshot.clone()).await;

    // Verify the stored snapshot is the one we swapped in
    let stored = mgr
        .get_config_snapshot()
        .await
        .expect("snapshot should exist");
    assert!(
        std::sync::Arc::ptr_eq(&stored, &snapshot),
        "stored snapshot should be the same Arc as the one we swapped in"
    );

    // Swap in a different snapshot
    let mut map2 = HashMap::new();
    map2.insert(ConfigSection::Models, serde_json::json!({"models": []}));
    let snapshot2: ConfigSnapshot = ConfigSnapshot::new(map2);
    mgr.swap_config_snapshot(snapshot2.clone()).await;

    let stored2 = mgr
        .get_config_snapshot()
        .await
        .expect("snapshot should exist");
    assert!(
        std::sync::Arc::ptr_eq(&stored2, &snapshot2),
        "stored snapshot should now be the second one"
    );
    assert!(
        !std::sync::Arc::ptr_eq(&stored2, &snapshot),
        "new snapshot should differ from the old one"
    );
}
