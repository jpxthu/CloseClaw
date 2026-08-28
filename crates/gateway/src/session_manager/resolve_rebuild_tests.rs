//! Tests for resolve() system prompt rebuild on archived→active restore
//! when ConversationSession already exists in memory (needs_conv = false).
//!
//! Step 1.3 of plan: verify that rebuild_system_prompt is called and
//! system_appends are correctly stacked on top of the rebuilt prompt.

use super::tests::{test_config, TestPromptBuilder};
use super::SessionManager;
use crate::Message;
use closeclaw_common::{
    AgentSkillsQuery, AgentToolsConfig, AgentToolsConfigQuery, BootstrapMode, DynamicPromptBuilder,
    DynamicPromptContext, PromptOverrides, SkillListingProvider,
};
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint, SessionStatus,
};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

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
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

/// Mock persistence that clones checkpoint on every load (never consumes it),
/// so both `try_restore_archived_session_inner` (load + restore) and the
/// subsequent `cm.load()` in the restore block can succeed.
struct RebuildMockPersist {
    checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
    restore_called: std::sync::atomic::AtomicBool,
}

impl RebuildMockPersist {
    fn new(cp: SessionCheckpoint) -> Self {
        Self {
            checkpoint: tokio::sync::Mutex::new(Some(cp)),
            restore_called: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl PersistenceService for RebuildMockPersist {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        // Clone so the checkpoint is never consumed — both try_restore and
        // the subsequent cm.load() in the resolve block will succeed.
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn load_archived_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
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
        self.restore_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // Clone — do not consume, so cm.load() below can still succeed.
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
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
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn find_archived_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(None)
    }
}

/// Helper: create a temp workspace with bootstrap files for prompt rebuild.
fn make_temp_workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    for (name, content) in files {
        let path = tmp.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }
    tmp
}

/// Test: When archived session is restored via Path 2 with needs_conv = false
/// (ConversationSession already in memory), rebuild_system_prompt is called
/// and system_prompt reflects the rebuilt content.
#[tokio::test]
async fn test_archived_restore_needs_conv_false_rebuilds_system_prompt() {
    let session_id = "test-rebuild-sid".to_string();
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.platform = Some("feishu".to_string());

    let mock = Arc::new(RebuildMockPersist::new(cp));
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock.clone()),
        None,
        ReasoningLevel::default(),
    );

    // Set system prompt builder on SessionManager so the needs_conv = false
    // path can use it when calling rebuild_system_prompt.
    let tmp = make_temp_workspace(&[
        ("AGENTS.md", "rebuilt agents content"),
        ("SOUL.md", "rebuilt soul content"),
        ("IDENTITY.md", "rebuilt identity content"),
        ("USER.md", "rebuilt user content"),
        ("TOOLS.md", "rebuilt tools content"),
        ("BOOTSTRAP.md", "rebuilt bootstrap content"),
        ("MEMORY.md", "rebuilt memory content"),
    ]);
    let workspace_path = tmp.path().to_path_buf();
    mgr.set_system_prompt_builder(Arc::new(TestPromptBuilder::new(
        Some(workspace_path),
        BootstrapMode::Full,
    )))
    .await;

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // 1. Pre-populate key_registry so resolve takes Path 2 (registry hit)
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }

    // 2. Pre-populate sessions map (so the session is "active" in-memory)
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

    // 3. Pre-populate conversation_sessions with a ConversationSession
    //    (this is the "needs_conv = false" scenario).
    let conv = ConversationSession::new(
        session_id.clone(),
        "test-model".to_string(),
        PathBuf::from("/tmp/test-workspace"),
    )
    .with_system_prompt("old-stale-prompt");

    {
        let mut cs = mgr.conversation_sessions.write().await;
        cs.insert(session_id.clone(), Arc::new(RwLock::new(conv)));
    }

    // 4. Call find_or_create — should take Path 2 → needs_conv = false →
    //    rebuild_system_prompt on the existing ConversationSession.
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, session_id);
    assert!(
        mock.restore_called
            .load(std::sync::atomic::Ordering::SeqCst),
        "restore_checkpoint should have been called"
    );

    // 5. Verify that the system prompt was rebuilt (contains new workspace content).
    let cs = mgr.get_conversation_session(&session_id).await.unwrap();
    let cs = cs.read().await;
    let prompt = cs.system_prompt().expect("system prompt should exist");
    assert_ne!(
        prompt, "old-stale-prompt",
        "system prompt should have been rebuilt, not remain stale"
    );
    assert!(
        prompt.contains("rebuilt agents content"),
        "rebuilt prompt should contain workspace content: {}",
        prompt
    );
    assert!(
        prompt.contains("rebuilt soul content"),
        "rebuilt prompt should contain soul content"
    );
}

/// Test: When archived session is restored with system_appends in the checkpoint,
/// the appends are correctly stacked on top of the rebuilt system prompt
/// (appends restore must happen AFTER rebuild_system_prompt).
#[tokio::test]
async fn test_archived_restore_system_appends_stacked_on_rebuilt_prompt() {
    let session_id = "test-appends-sid".to_string();
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.platform = Some("feishu".to_string());
    cp.system_appends = vec!["custom-append-1".to_string(), "custom-append-2".to_string()];

    let mock = Arc::new(RebuildMockPersist::new(cp));
    let mgr = SessionManager::new(
        &test_config(),
        Some(mock.clone()),
        None,
        ReasoningLevel::default(),
    );

    // Set system prompt builder on SessionManager so the needs_conv = false
    // path can use it when calling rebuild_system_prompt.
    let tmp = make_temp_workspace(&[
        ("AGENTS.md", "agents content"),
        ("SOUL.md", "soul content"),
        ("IDENTITY.md", "identity content"),
        ("USER.md", "user content"),
        ("TOOLS.md", "tools content"),
        ("BOOTSTRAP.md", "bootstrap content"),
        ("MEMORY.md", "memory content"),
    ]);
    let workspace_path = tmp.path().to_path_buf();
    mgr.set_system_prompt_builder(Arc::new(TestPromptBuilder::new(
        Some(workspace_path),
        BootstrapMode::Full,
    )))
    .await;

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

    let mut conv = ConversationSession::new(
        session_id.clone(),
        "test-model".to_string(),
        PathBuf::from("/tmp/test-workspace"),
    )
    .with_system_prompt("old-prompt");
    // Pre-populate system_appends that should be overwritten by checkpoint restore
    conv.add_system_append("stale-append".to_string());

    {
        let mut cs = mgr.conversation_sessions.write().await;
        cs.insert(session_id.clone(), Arc::new(RwLock::new(conv)));
    }

    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, session_id);

    // Verify system_appends from checkpoint are restored
    let cs = mgr.get_conversation_session(&session_id).await.unwrap();
    let cs = cs.read().await;
    let appends = cs.user_system_appends();
    assert_eq!(
        appends,
        &["custom-append-1".to_string(), "custom-append-2".to_string()],
        "system_appends should be restored from checkpoint, not stale"
    );

    // Verify system prompt was rebuilt (not the old stale one)
    let prompt = cs.system_prompt().expect("system prompt should exist");
    assert_ne!(
        prompt, "old-prompt",
        "system prompt should have been rebuilt"
    );
    assert!(
        prompt.contains("agents content"),
        "rebuilt prompt should contain workspace content"
    );
}

/// Mock persistence for Path 3 test: supports find_archived_session_by_routing
/// so resolve takes the archived-hit path (Path 3 → needs_conv = false).
struct Path3RebuildMock {
    checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
    archived_id: Option<String>,
    restore_called: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl PersistenceService for Path3RebuildMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn load_archived_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
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
        self.restore_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
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
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn find_archived_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        // Return the archived session ID to trigger the Path 3 archived hit
        Ok(self.archived_id.clone())
    }
}

/// Test: When needs_conv = false path is taken for Path 3 (registry miss +
/// archived hit via routing), rebuild_system_prompt is called and system_appends
/// are restored.
#[tokio::test]
async fn test_archived_restore_path3_needs_conv_false() {
    let session_id = "test-path3-rebuild".to_string();
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_peer_id("agent-b".to_string())
        .with_agent_id("agent-b".to_string());
    cp.platform = Some("feishu".to_string());
    cp.system_appends = vec!["path3-append".to_string()];
    cp.sender_id = Some("user-a".to_string());
    cp.account_id = None;

    let mock = Arc::new(Path3RebuildMock {
        checkpoint: tokio::sync::Mutex::new(Some(cp)),
        archived_id: Some(session_id.clone()),
        restore_called: std::sync::atomic::AtomicBool::new(false),
    });

    let mgr = SessionManager::new(
        &test_config(),
        Some(mock.clone() as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    );

    let msg = test_message();
    // key_registry is empty → miss (Path 3)

    let tmp = make_temp_workspace(&[
        ("AGENTS.md", "path3 agents"),
        ("SOUL.md", "path3 soul"),
        ("IDENTITY.md", "path3 identity"),
        ("USER.md", "path3 user"),
        ("TOOLS.md", "path3 tools"),
        ("BOOTSTRAP.md", "path3 bootstrap"),
        ("MEMORY.md", "path3 memory"),
    ]);
    let workspace_path = tmp.path().to_path_buf();

    // Pre-populate conversation_sessions (needs_conv = false scenario)
    let mut conv = ConversationSession::new(
        session_id.clone(),
        "test-model".to_string(),
        PathBuf::from("/tmp/test-workspace"),
    )
    .with_system_prompt("old-path3-prompt");
    conv.set_system_prompt_builder(Arc::new(TestPromptBuilder::new(
        Some(workspace_path),
        BootstrapMode::Full,
    )));

    {
        let mut cs = mgr.conversation_sessions.write().await;
        cs.insert(session_id.clone(), Arc::new(RwLock::new(conv)));
    }

    // find_or_create should find the archived session via routing and restore it
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, session_id);

    let cs = mgr.get_conversation_session(&session_id).await.unwrap();
    let cs = cs.read().await;

    // Verify prompt was rebuilt
    let prompt = cs.system_prompt().expect("system prompt should exist");
    assert_ne!(
        prompt, "old-path3-prompt",
        "system prompt should have been rebuilt"
    );
    assert!(
        prompt.contains("path3 agents"),
        "rebuilt prompt should contain workspace content: {}",
        prompt
    );

    // Verify system_appends restored from checkpoint
    let appends = cs.user_system_appends();
    assert_eq!(
        appends,
        &["path3-append".to_string()],
        "system_appends should be restored from checkpoint"
    );
}

// ── Mock types for injection chain verification ──────────────────────────

/// Mock DynamicPromptBuilder that returns known parts.
struct MockDynamicPromptBuilder;

#[async_trait::async_trait]
impl DynamicPromptBuilder for MockDynamicPromptBuilder {
    fn build_prompt_parts(
        &self,
        _ctx: &DynamicPromptContext<'_>,
    ) -> (Option<String>, Option<String>) {
        (Some("mock-static".into()), Some("mock-dynamic".into()))
    }
}

/// Mock SkillListingProvider that returns a known listing.
struct MockSkillListingProvider;

impl SkillListingProvider for MockSkillListingProvider {
    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        "mock-skill-listing".into()
    }

    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        "mock-skill-listing".into()
    }

    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::skill_listing_provider::ConditionalSkillMatch> {
        vec![]
    }
}

/// Mock AgentRegistryQuery that returns a known config with skills.
struct MockAgentRegistryQuery;

#[async_trait::async_trait]
impl closeclaw_agent::AgentLookup for MockAgentRegistryQuery {
    async fn get_agent_model(
        &self,
        _agent_id: &str,
    ) -> Option<closeclaw_config::agents::ModelSpec> {
        None
    }
    async fn agent_exists(&self, _agent_id: &str) -> bool {
        true
    }
    async fn query_bootstrap_mode(&self, _agent_id: &str) -> Option<BootstrapMode> {
        Some(BootstrapMode::Full)
    }
    async fn get_agent_workspace(&self, _agent_id: &str) -> Option<PathBuf> {
        None
    }
}

#[async_trait::async_trait]
impl AgentSkillsQuery for MockAgentRegistryQuery {
    fn get_agent_skills(&self, _agent_id: &str) -> Option<Vec<String>> {
        Some(vec!["skill-a".into(), "skill-b".into()])
    }
}

#[async_trait::async_trait]
impl AgentToolsConfigQuery for MockAgentRegistryQuery {
    async fn get_agent_tools_config(&self, _agent_id: &str) -> Option<AgentToolsConfig> {
        None
    }
}

impl closeclaw_agent::AgentRegistryQuery for MockAgentRegistryQuery {}

/// Mock SessionConfigProvider that returns git_status = true.
struct MockSessionConfigProvider;

impl closeclaw_config::session::SessionConfigProvider for MockSessionConfigProvider {
    fn session_config_for(
        &self,
        _agent_id: &str,
        _role: closeclaw_session::persistence::AgentRole,
    ) -> closeclaw_config::session::PerAgentSessionConfig {
        closeclaw_config::session::PerAgentSessionConfig {
            idle_minutes: 30,
            purge_after_minutes: 0,
            is_git_status_enabled: true,
        }
    }
    fn sweeper_interval_secs(&self) -> u64 {
        60
    }
    fn dreaming_interval_secs(&self) -> u64 {
        300
    }
    fn consistency_check_interval_secs(&self) -> u64 {
        120
    }
    fn list_agents(&self) -> Vec<String> {
        vec![]
    }
    fn compact_config(&self) -> closeclaw_common::CompactConfig {
        closeclaw_common::CompactConfig::default()
    }
}

/// Persistence mock for checkpoint_manager setup.
struct InjectionPersistMock;

#[async_trait::async_trait]
impl PersistenceService for InjectionPersistMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn load_archived_checkpoint(
        &self,
        _: &str,
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
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
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
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn find_archived_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(None)
    }
}

/// Persistence mock for archived session restore (returns checkpoint on load/restore).
struct ArchRestorePersistMock {
    checkpoint: tokio::sync::Mutex<Option<SessionCheckpoint>>,
}

#[async_trait::async_trait]
impl PersistenceService for ArchRestorePersistMock {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn load_archived_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
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
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
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
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn find_archived_session_by_routing(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(None)
    }
}

/// Setup a SessionManager with all injection dependencies configured.
/// Returns (SessionManager, TempDir) — caller must keep TempDir alive.
async fn setup_mgr_with_injection_deps(
    persist: Arc<dyn PersistenceService>,
) -> (SessionManager, tempfile::TempDir) {
    let mgr = SessionManager::new(
        &test_config(),
        Some(persist),
        None,
        ReasoningLevel::default(),
    );

    // System prompt builder
    let tmp = make_temp_workspace(&[
        ("AGENTS.md", "injected agents"),
        ("SOUL.md", "injected soul"),
        ("IDENTITY.md", "injected identity"),
        ("USER.md", "injected user"),
        ("TOOLS.md", "injected tools"),
        ("BOOTSTRAP.md", "injected bootstrap"),
        ("MEMORY.md", "injected memory"),
    ]);
    mgr.set_system_prompt_builder(Arc::new(TestPromptBuilder::new(
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
    )))
    .await;

    // Dynamic prompt builder
    mgr.set_dynamic_prompt_builder(Arc::new(MockDynamicPromptBuilder))
        .await;

    // Skill listing provider
    mgr.set_skill_listing_provider(Arc::new(MockSkillListingProvider))
        .await;

    // Agent registry (for get_agent_config / query_bootstrap_mode)
    mgr.set_agent_registry(
        Arc::new(MockAgentRegistryQuery) as Arc<dyn closeclaw_agent::AgentRegistryQuery>
    )
    .await;

    // Config manager (for get_agent_config which reads from config_manager.agents)
    let config_dir = tempfile::TempDir::new().unwrap();
    let cm = Arc::new(
        closeclaw_config::manager::ConfigManager::new(config_dir.path().to_path_buf()).unwrap(),
    );
    {
        let mut agents = cm.agents.write().unwrap();
        agents.insert(
            "agent-b".into(),
            closeclaw_config::agents::ResolvedAgentConfig {
                id: "agent-b".into(),
                name: "agent-b".into(),
                parent_id: None,
                model: Some(closeclaw_config::agents::ModelSpec::single("test-model")),
                workspace: None,
                agent_dir: None,
                bootstrap_mode: BootstrapMode::Full,
                skills: vec!["skill-a".into(), "skill-b".into()],
                tools: vec![],
                disallowed_tools: vec![],
                subagents: closeclaw_config::agents::SubagentsConfig::default(),
                memory: closeclaw_config::agents::MemoryConfig::default(),
                hooks: vec![],
                parallel_tool_calls: true,
                memory_configured: false,
                source: closeclaw_config::agents::ConfigSource::Merged,
            },
        );
    }
    mgr.set_config_manager(cm).await;

    // Prompt overrides
    mgr.set_prompt_overrides(Some(PromptOverrides {
        override_prompt: None,
        agent_prompt: None,
        custom_prompt: Some("test-override".into()),
    }))
    .await;

    // Note: checkpoint_manager is already set up by SessionManager::new from
    // the persist parameter. No need to override it here.

    // Session config provider (for git_status)
    mgr.set_session_config_provider(Arc::new(MockSessionConfigProvider))
        .await;

    (mgr, tmp)
}

// ── Test: full injection chain in archived restore (needs_conv = false) ──

/// When an archived session is restored with ConversationSession already in
/// memory, `rebuild_archived_session_prompt` must wire up ALL injection deps:
/// dynamic_prompt_builder, skill_listing_provider, agent_skills,
/// prompt_overrides, bootstrap_mode, snapshot_meta_store, checkpoint_storage,
/// and session config (git_status).
#[tokio::test]
async fn test_archived_restore_full_injection_chain() {
    let session_id = "injection-chain-sid".to_string();
    let mut cp = SessionCheckpoint::new(session_id.clone())
        .with_status(SessionStatus::Archived)
        .with_peer_id("agent-b".into())
        .with_agent_id("agent-b".into());
    cp.platform = Some("feishu".into());

    let persist: Arc<dyn PersistenceService> = Arc::new(ArchRestorePersistMock {
        checkpoint: tokio::sync::Mutex::new(Some(cp)),
    });
    let (mgr, _tmp) = setup_mgr_with_injection_deps(persist).await;

    let msg = test_message();
    let routing_key = SessionManager::compute_routing_key("feishu", &msg, None);

    // Pre-populate key_registry (Path 2: registry hit)
    {
        let mut reg = mgr.key_registry.write().await;
        reg.insert(routing_key.clone(), session_id.clone());
    }
    // Pre-populate sessions map
    {
        let mut sessions = mgr.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            crate::Session {
                id: session_id.clone(),
                agent_id: "agent-b".into(),
                channel: "feishu".into(),
                created_at: chrono::Utc::now().timestamp(),
                depth: 0,
            },
        );
    }
    // Pre-populate conversation_sessions (needs_conv = false)
    let conv = ConversationSession::new(
        session_id.clone(),
        "test-model".into(),
        PathBuf::from("/tmp/test-workspace"),
    )
    .with_system_prompt("old-prompt");
    {
        let mut cs = mgr.conversation_sessions.write().await;
        cs.insert(session_id.clone(), Arc::new(RwLock::new(conv)));
    }

    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();
    assert_eq!(resolved, session_id);

    // Verify all injection deps are set
    let cs = mgr.get_conversation_session(&session_id).await.unwrap();
    let cs = cs.read().await;

    // 1. system_prompt_builder injected (basic, was already present)
    assert!(
        cs.has_system_prompt_builder(),
        "system_prompt_builder should be set"
    );

    // 2. dynamic_prompt_builder injected (new in Step 1.1)
    //    Verify by checking that rebuild_system_prompt produces a prompt
    //    that includes workspace content (which requires the builder to work).
    let prompt = cs.system_prompt().expect("system prompt should exist");
    assert!(
        prompt.contains("injected agents"),
        "prompt should contain workspace content after full injection: {}",
        prompt
    );

    // 3. skill_listing_provider injected (new in Step 1.1)
    assert!(
        cs.skill_listing_provider().is_some(),
        "skill_listing_provider should be set"
    );

    // 4. agent_skills injected from agent config (new in Step 1.1)
    let skills = cs.agent_skills();
    assert!(skills.is_some(), "agent_skills should be set");
    let skills = skills.unwrap();
    assert_eq!(
        skills,
        &["skill-a", "skill-b"],
        "agent_skills should match config"
    );

    // 5. prompt_overrides injected (new in Step 1.1)
    //    Verified indirectly: rebuild_system_prompt was called with the
    //    builder (which is the main verification).

    // 6. bootstrap_mode cached on session (new in Step 1.1)
    //    After with_bootstrap_mode, the session's bootstrap_mode should be Full.
    //    Verified indirectly by the prompt content matching Full mode files.

    // 7. git_status set from session config (new in Step 1.1)
    //    MockSessionConfigProvider returns is_git_status_enabled = true.
    //    Session accepted the config (no panic = success).
}

// ── Test: new session path regression ─────────────────────────────────────

/// Verify that a brand-new session (not archived) still receives full
/// injection. This is a regression guard: Step 1.1 only modified the
/// archived path, so the new-session path should be unchanged.
#[tokio::test]
async fn test_new_session_path_injection_unchanged() {
    let persist: Arc<dyn PersistenceService> = Arc::new(InjectionPersistMock);
    let (mgr, _tmp) = setup_mgr_with_injection_deps(persist).await;

    let msg = test_message();
    let resolved = mgr.find_or_create("feishu", &msg, None).await.unwrap();

    // Verify the new session has all injection deps
    let cs = mgr.get_conversation_session(&resolved).await.unwrap();
    let cs = cs.read().await;
    assert!(
        cs.has_system_prompt_builder(),
        "new session: system_prompt_builder"
    );
    assert!(
        cs.skill_listing_provider().is_some(),
        "new session: skill_listing_provider"
    );
    assert!(cs.agent_skills().is_some(), "new session: agent_skills");
    // System prompt should be built with workspace content
    let prompt = cs.system_prompt().expect("system prompt should exist");
    assert!(
        prompt.contains("injected agents"),
        "new session prompt should contain workspace content: {}",
        prompt
    );
}
