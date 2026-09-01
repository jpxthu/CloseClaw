//! Tests for resolve path config queries via AgentRegistry.
//!
//! Covers the resolve path behavior when querying workspace and bootstrap_mode
//! through `AgentRegistryQuery` instead of `ConfigManager`:
//! - Workspace: per-agent workspace from registry / fallback to global / None fallback
//! - Bootstrap mode: from registry / default Full when agent not found
//! - Agent not found: graceful fallback

use super::tests::{make_test_mgr, test_config};
use super::SessionManager;
use crate::Message;
use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::ModelSpec;
use closeclaw_session::persistence::ReasoningLevel;
use std::path::PathBuf;
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

// ── Configurable mock for per-agent responses ──────────────────────────────

/// Mock that returns different values per agent_id, enabling per-agent
/// workspace, skills, tools, and bootstrap_mode configuration.
struct PerAgentMock {
    workspaces: std::collections::HashMap<String, Option<PathBuf>>,
    bootstrap_modes: std::collections::HashMap<String, BootstrapMode>,
    default_workspace: Option<PathBuf>,
    default_bootstrap_mode: BootstrapMode,
}

impl PerAgentMock {
    fn new() -> Self {
        Self {
            workspaces: std::collections::HashMap::new(),
            bootstrap_modes: std::collections::HashMap::new(),
            default_workspace: None,
            default_bootstrap_mode: BootstrapMode::Full,
        }
    }

    fn with_workspace(mut self, agent_id: &str, ws: Option<PathBuf>) -> Self {
        self.workspaces.insert(agent_id.to_string(), ws);
        self
    }

    fn with_bootstrap_mode(mut self, agent_id: &str, mode: BootstrapMode) -> Self {
        self.bootstrap_modes.insert(agent_id.to_string(), mode);
        self
    }
}

#[async_trait::async_trait]
impl closeclaw_agent::AgentLookup for PerAgentMock {
    async fn get_agent_model(&self, _agent_id: &str) -> Option<ModelSpec> {
        None
    }
    async fn agent_exists(&self, _agent_id: &str) -> bool {
        true
    }
    async fn get_agent_workspace(&self, agent_id: &str) -> Option<PathBuf> {
        self.workspaces
            .get(agent_id)
            .cloned()
            .unwrap_or_else(|| self.default_workspace.clone())
    }
    async fn query_bootstrap_mode(&self, agent_id: &str) -> Option<BootstrapMode> {
        Some(
            self.bootstrap_modes
                .get(agent_id)
                .copied()
                .unwrap_or(self.default_bootstrap_mode),
        )
    }
}

#[async_trait::async_trait]
impl closeclaw_common::AgentSkillsQuery for PerAgentMock {
    fn get_agent_skills(&self, _agent_id: &str) -> Option<Vec<String>> {
        None
    }
}

#[async_trait::async_trait]
impl closeclaw_common::AgentToolsConfigQuery for PerAgentMock {
    async fn get_agent_tools_config(
        &self,
        _agent_id: &str,
    ) -> Option<closeclaw_common::AgentToolsConfig> {
        None
    }
}

impl closeclaw_agent::AgentRegistryQuery for PerAgentMock {}

// ── Workspace fallback tests ──────────────────────────────────────────────

/// When AgentRegistry returns a per-agent workspace, the session workdir
/// should be derived from it (not the global workspace_dir).
#[tokio::test]
async fn test_resolve_uses_per_agent_workspace() {
    let agent_ws = tempfile::TempDir::new().unwrap();
    let mock = PerAgentMock::new().with_workspace("agent-b", Some(agent_ws.path().to_path_buf()));

    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    mgr.set_agent_registry(Arc::new(mock)).await;
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let workdir = conv.workdir();
    // Workdir should be derived from per-agent workspace
    assert!(
        workdir.starts_with(agent_ws.path()),
        "workdir should be under per-agent workspace: {:?}",
        workdir
    );
}

/// When AgentRegistry returns None for workspace and global workspace_dir
/// is set, the session should use the global workspace.
#[tokio::test]
async fn test_resolve_falls_back_to_global_workspace() {
    let global_ws = tempfile::TempDir::new().unwrap();
    // AgentRegistry returns None for this agent
    let mock = PerAgentMock::new().with_workspace("agent-b", None);

    let mgr = SessionManager::new(
        &test_config(),
        None,
        Some(global_ws.path().to_path_buf()),
        ReasoningLevel::default(),
    );
    mgr.set_agent_registry(Arc::new(mock)).await;
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let workdir = conv.workdir();
    // Workdir should be derived from global workspace
    assert!(
        workdir.starts_with(global_ws.path()),
        "workdir should be under global workspace: {:?}",
        workdir
    );
}

/// When both AgentRegistry and global workspace_dir are None,
/// the session should fall back to /tmp.
#[tokio::test]
async fn test_resolve_no_workspace_falls_back_to_tmp() {
    let mock = PerAgentMock::new().with_workspace("agent-b", None);

    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    mgr.set_agent_registry(Arc::new(mock)).await;
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    let workdir = conv.workdir();
    assert_eq!(workdir, PathBuf::from("/tmp"));
}

// ── Bootstrap mode from registry tests ─────────────────────────────────────

/// Bootstrap mode from AgentRegistry should be cached in ConversationSession.
#[tokio::test]
async fn test_resolve_caches_bootstrap_mode_full() {
    let mock = PerAgentMock::new().with_bootstrap_mode("agent-b", BootstrapMode::Full);

    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    mgr.set_agent_registry(Arc::new(mock)).await;
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    assert_eq!(conv.bootstrap_mode(), BootstrapMode::Full);
}

#[tokio::test]
async fn test_resolve_caches_bootstrap_mode_minimal() {
    let mock = PerAgentMock::new().with_bootstrap_mode("agent-b", BootstrapMode::Minimal);

    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    mgr.set_agent_registry(Arc::new(mock)).await;
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    assert_eq!(conv.bootstrap_mode(), BootstrapMode::Minimal);
}

/// When no registry is set, bootstrap_mode defaults to Full.
#[tokio::test]
async fn test_resolve_no_registry_defaults_to_full() {
    let mgr = make_test_mgr(None);
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    assert_eq!(conv.bootstrap_mode(), BootstrapMode::Full);
}

// ── Agent not found: graceful fallback ─────────────────────────────────────

/// Mock that returns None for bootstrap_mode (agent not found).
struct NotFoundMock;

#[async_trait::async_trait]
impl closeclaw_agent::AgentLookup for NotFoundMock {
    async fn get_agent_model(&self, _agent_id: &str) -> Option<ModelSpec> {
        None
    }
    async fn agent_exists(&self, _agent_id: &str) -> bool {
        false
    }
    async fn get_agent_workspace(&self, _agent_id: &str) -> Option<PathBuf> {
        None
    }
    async fn query_bootstrap_mode(&self, _agent_id: &str) -> Option<BootstrapMode> {
        None
    }
}

#[async_trait::async_trait]
impl closeclaw_common::AgentSkillsQuery for NotFoundMock {
    fn get_agent_skills(&self, _agent_id: &str) -> Option<Vec<String>> {
        None
    }
}

#[async_trait::async_trait]
impl closeclaw_common::AgentToolsConfigQuery for NotFoundMock {
    async fn get_agent_tools_config(
        &self,
        _agent_id: &str,
    ) -> Option<closeclaw_common::AgentToolsConfig> {
        None
    }
}

impl closeclaw_agent::AgentRegistryQuery for NotFoundMock {}

/// When agent is not found in registry, bootstrap_mode falls back to Full.
#[tokio::test]
async fn test_resolve_agent_not_found_bootstrap_defaults_to_full() {
    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    mgr.set_agent_registry(Arc::new(NotFoundMock)).await;
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    // Agent not found → query_bootstrap_mode returns None → defaults to Full
    assert_eq!(conv.bootstrap_mode(), BootstrapMode::Full);
}

/// When agent is not found, workspace falls back to global or /tmp.
#[tokio::test]
async fn test_resolve_agent_not_found_workspace_fallback() {
    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    mgr.set_agent_registry(Arc::new(NotFoundMock)).await;
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    // No registry workspace + no global workspace → /tmp
    assert_eq!(conv.workdir(), PathBuf::from("/tmp"));
}

// ── Bootstrap mode propagation to rebuild_system_prompt ────────────────────

/// Verify that rebuild_system_prompt_for_session uses the cached
/// bootstrap_mode from ConversationSession.
#[tokio::test]
async fn test_rebuild_system_prompt_uses_cached_bootstrap_mode() {
    let mock = PerAgentMock::new().with_bootstrap_mode("agent-b", BootstrapMode::Minimal);

    let mgr = SessionManager::new(&test_config(), None, None, ReasoningLevel::default());
    mgr.set_agent_registry(Arc::new(mock)).await;
    let msg = test_message();

    let session_id = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    // Session was created with Minimal bootstrap_mode from registry
    {
        let conv = mgr.get_conversation_session(&session_id).await.unwrap();
        let conv = conv.read().await;
        assert_eq!(conv.bootstrap_mode(), BootstrapMode::Minimal);
    }

    // rebuild_system_prompt_for_session should use cached mode
    mgr.rebuild_system_prompt_for_session(&session_id).await;

    // Verify bootstrap_mode is still Minimal (unchanged by rebuild)
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv = conv.read().await;
    assert_eq!(conv.bootstrap_mode(), BootstrapMode::Minimal);
}

// ── Archived session restore: reply_ref propagation ─────────────────────

/// When an archived session is restored, the checkpoint should be saved with
/// the new message's reply_ref (Step 1.13 fix for archived restore path).
#[tokio::test]
async fn test_archived_session_restore_sets_reply_ref() {
    use super::test_helpers::MockPersistService;
    use closeclaw_session::persistence::{SessionCheckpoint, SessionStatus};

    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: tokio::sync::Mutex::new(Some(
            SessionCheckpoint::new("archived-sid".to_string())
                .with_status(SessionStatus::Archived)
                .with_peer_id("agent-b".to_string()),
        )),
        restore_called: tokio::sync::Mutex::new(false),
        saved_checkpoint: tokio::sync::Mutex::new(None),
    });
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock_storage.clone()),
        None,
        ReasoningLevel::default(),
    );
    // Message with reply_ref set
    let msg = Message {
        id: "msg-2".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        reply_ref: Some("om_reply_target".to_string()),
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    // Insert routing_key so resolve hits Path 2 (archived restore from registry)
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key, "archived-sid".to_string());
    }
    let result = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(result, "archived-sid");
    // Verify the saved checkpoint has reply_ref from the new message
    let saved = mock_storage.saved_checkpoint.lock().await;
    let cp = saved.as_ref().expect("checkpoint should have been saved");
    assert_eq!(
        cp.reply_ref,
        Some("om_reply_target".to_string()),
        "archived session restore should propagate reply_ref to checkpoint"
    );
}
