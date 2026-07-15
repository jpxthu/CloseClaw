use super::*;
use crate::{GatewayConfig, Message};
use closeclaw_common::{BootstrapMode, PromptOverrides, SystemPromptBuilder};
use closeclaw_config::manager::ConfigSnapshot;
use closeclaw_session::persistence::SessionCheckpoint;
use serial_test::serial;
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::sync::Mutex;

/// Clear section cache. No-op: the global SECTION_CACHE was removed during
/// Step 1.5 refactor. Kept for call-site compatibility.
pub(crate) fn clear_global_prompt_state() {}

/// Mock SystemPromptBuilder that reads bootstrap files from a workspace directory.
/// Used by tests that need to verify workspace file injection into system prompts.
struct TestPromptBuilder {
    workspace_dir: Option<PathBuf>,
    bootstrap_mode: BootstrapMode,
}

impl TestPromptBuilder {
    fn new(workspace_dir: Option<PathBuf>, bootstrap_mode: BootstrapMode) -> Self {
        Self {
            workspace_dir,
            bootstrap_mode,
        }
    }
}

#[async_trait::async_trait]
impl SystemPromptBuilder for TestPromptBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        bootstrap_mode_override: Option<BootstrapMode>,
    ) -> String {
        let mode = bootstrap_mode_override.unwrap_or(self.bootstrap_mode);
        let Some(ref workspace) = self.workspace_dir else {
            return String::new();
        };
        let files =
            closeclaw_session::bootstrap::load_bootstrap_files(workspace, mode).unwrap_or_default();
        let mut parts: Vec<String> = files.into_iter().map(|(_, v)| v).collect();
        parts.sort();
        parts.join("\n")
    }

    async fn invalidate_cache(&self) {}
}

pub(crate) fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
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
        ReasoningLevel::default(),
    );
    let msg = test_message();
    // Populate key_registry so resolve can look up the session.
    // resolve() computes routing_key from message fields — insert it.
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);
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
pub(crate) fn make_test_mgr(workspace: Option<&std::path::Path>) -> SessionManager {
    SessionManager::new(
        &test_config(),
        None,
        workspace.map(std::path::PathBuf::from),
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
        ReasoningLevel::default(),
    );
    mgr.set_system_prompt_builder(Arc::new(TestPromptBuilder::new(
        workspace_dir,
        BootstrapMode::Full,
    )))
    .await;
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
        workspace_dir.clone(),
        ReasoningLevel::default(),
    );
    mgr.set_agent_registry(Arc::new(MockAgentRegistryQuery {
        bootstrap_mode: BootstrapMode::Minimal,
    }))
    .await;
    mgr.set_system_prompt_builder(Arc::new(TestPromptBuilder::new(
        workspace_dir,
        BootstrapMode::Minimal,
    )))
    .await;
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
    let workspace_path = tmp.path().to_path_buf();
    let mgr = SessionManager::new(
        &test_config(),
        None,
        Some(workspace_path.clone()),
        ReasoningLevel::default(),
    );
    mgr.set_system_prompt_builder(Arc::new(TestPromptBuilder::new(
        Some(workspace_path),
        BootstrapMode::Full,
    )))
    .await;
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

// DISABLED: references non-existent types (DiskSkillRegistry, ToolRegistry, SpawnController,
// BuiltinToolContext, AgentRegistry) that were removed during the common-crate decoupling.
// TODO: re-implement with the current trait-based architecture.
// #[tokio::test]
// #[serial]
// async fn test_find_or_create_with_tool_registry() {
// }

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

use super::test_helpers::MockAgentRegistryQuery;
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

// =====================================================================
// Step 1.7 — force_new_for_channel injection test
// =====================================================================

/// force_new_for_channel with a builder should produce a non-empty
/// system prompt after session creation.
#[tokio::test]
#[serial]
async fn test_force_new_injects_system_prompt_with_builder() {
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
    let workspace_dir = tmp.path().to_path_buf();
    let mgr = SessionManager::new(
        &test_config(),
        None,
        Some(workspace_dir.clone()),
        ReasoningLevel::default(),
    );
    mgr.set_system_prompt_builder(Arc::new(TestPromptBuilder::new(
        Some(workspace_dir),
        BootstrapMode::Full,
    )))
    .await;

    let session_id = mgr.force_new_for_channel("feishu", "agent-test").await;
    let conv_slot = mgr.get_conversation_session(&session_id).await;
    assert!(conv_slot.is_some(), "session should exist");
    let conv = conv_slot.unwrap();
    let conv = conv.read().await;
    let prompt = conv
        .system_prompt()
        .expect("system prompt should be non-empty");
    assert!(!prompt.is_empty(), "system prompt should be non-empty");
    assert!(
        prompt.contains("agents content"),
        "system prompt should contain bootstrap content"
    );
}
