//! Step 1.5 — Spawn timing tests: role marking before prompt build.
//!
//! Validates that `configure_spawn_behavior` calls `set_sub_agent(true)`
//! before `rebuild_system_prompt`, so the builder receives `SessionRole::Sub`.

use std::sync::Arc;

use closeclaw_common::{BootstrapMode, PromptOverrides, SessionRole, SystemPromptBuilder};
use closeclaw_config::agents::ResolvedAgentConfig;
use tokio::sync::RwLock;

use super::context::SpawnCreationContext;
use super::creation::{create_child_conversation_session, ChildSessionCreationParams};
use super::types::SpawnMode;
use crate::llm_session::ConversationSession;
use crate::persistence::{ReasoningLevel, SessionCheckpoint};

// ── Mock implementation (minimal, only what spawn timing tests need) ──────

struct MockCreationContext {
    parent_session: Arc<RwLock<ConversationSession>>,
    config_dir: std::path::PathBuf,
    system_prompt_builder: Option<Arc<dyn SystemPromptBuilder>>,
}

impl MockCreationContext {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cs = ConversationSession::new(
            "parent-session".to_string(),
            "test-model".to_string(),
            tmp.path().to_path_buf(),
        );
        let config_dir = tmp.path().join("config");
        Self {
            parent_session: Arc::new(RwLock::new(cs)),
            config_dir,
            system_prompt_builder: None,
        }
    }

    fn with_builder(builder: Arc<dyn SystemPromptBuilder>) -> Self {
        let mut ctx = Self::new();
        ctx.system_prompt_builder = Some(builder);
        ctx
    }
}

#[async_trait::async_trait]
impl SpawnCreationContext for MockCreationContext {
    async fn get_parent_conversation_session(
        &self,
        _parent_session_id: &str,
    ) -> Option<Arc<RwLock<ConversationSession>>> {
        Some(self.parent_session.clone())
    }

    async fn load_checkpoint(&self, _session_id: &str) -> Option<SessionCheckpoint> {
        None
    }

    async fn save_checkpoint(&self, _cp: &SessionCheckpoint) {}

    fn get_agent_config(&self, _agent_id: &str) -> Option<ResolvedAgentConfig> {
        None
    }

    fn shutdown_signal(&self) -> Option<Arc<dyn closeclaw_common::ShutdownSignal>> {
        None
    }

    fn default_reasoning_level(&self) -> ReasoningLevel {
        ReasoningLevel::default()
    }

    fn llm_caller(&self) -> Option<Arc<dyn closeclaw_common::LlmCaller>> {
        None
    }

    fn system_prompt_builder(&self) -> Option<Arc<dyn SystemPromptBuilder>> {
        self.system_prompt_builder.clone()
    }

    fn prompt_overrides(&self) -> Option<PromptOverrides> {
        None
    }

    fn dynamic_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::DynamicPromptBuilder>> {
        None
    }

    fn skill_listing_provider(&self) -> Option<Arc<dyn closeclaw_common::SkillListingProvider>> {
        None
    }

    async fn sender_id(&self, _session_id: &str) -> Option<String> {
        Some("test-user".to_string())
    }

    fn config_dir(&self) -> &std::path::Path {
        &self.config_dir
    }
}

fn make_config(id: &str) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: None,
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: Default::default(),
        memory: Default::default(),
        hooks: Vec::new(),
        parallel_tool_calls: true,
        memory_configured: false,
        source: closeclaw_config::agents::ConfigSource::User,
    }
}

fn default_params<'a>() -> ChildSessionCreationParams<'a> {
    ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 0,
        task: "test task",
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: false,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: None,
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
        debug_log: None,
        trace_id: "",
        session_key: None,
    }
}

// ── Mock builder that records session_role ────────────────────────────────

struct RoleCapturingBuilder {
    captured: Arc<tokio::sync::Mutex<Option<SessionRole>>>,
}

#[async_trait::async_trait]
impl SystemPromptBuilder for RoleCapturingBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<BootstrapMode>,
        session_role: SessionRole,
    ) -> String {
        *self.captured.lock().await = Some(session_role);
        "child-prompt".to_string()
    }

    async fn invalidate_cache(&self) {}
}

// ── Tests ────────────────────────────────────────────────────────────────

/// The child session must be marked as sub_agent BEFORE the system prompt
/// is built, so that the builder receives SessionRole::Sub.
///
/// This validates the timing fix in Step 1.4: `set_sub_agent(true)` is
/// called before `rebuild_system_prompt`, not after.
#[tokio::test]
async fn test_spawn_child_session_role_sub_before_prompt_build() {
    let captured = Arc::new(tokio::sync::Mutex::new(None::<SessionRole>));
    let builder: Arc<dyn SystemPromptBuilder> = Arc::new(RoleCapturingBuilder {
        captured: captured.clone(),
    });
    let ctx = MockCreationContext::with_builder(builder);
    let config = make_config("child-agent");
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    // 1. Child session must be marked as sub-agent.
    let cs = result.conversation_session.read().await;
    assert!(
        cs.is_sub_agent(),
        "child session must be marked as sub_agent"
    );

    // 2. Builder must have received SessionRole::Sub.
    let role = captured.lock().await;
    assert_eq!(
        *role,
        Some(SessionRole::Sub),
        "system prompt builder must receive Sub role for child session"
    );
}

/// Even when bootstrap_mode_override is Full, the child session role must
/// be Sub — role is orthogonal to bootstrap_mode.
#[tokio::test]
async fn test_spawn_child_role_orthogonal_to_bootstrap_mode() {
    let captured = Arc::new(tokio::sync::Mutex::new(None::<SessionRole>));
    let builder: Arc<dyn SystemPromptBuilder> = Arc::new(RoleCapturingBuilder {
        captured: captured.clone(),
    });
    let ctx = MockCreationContext::with_builder(builder);
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        light_context: false,
        ..default_params()
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    let cs = result.conversation_session.read().await;
    assert!(cs.is_sub_agent());

    // Child always uses Minimal bootstrap_mode (resolve_bootstrap_mode),
    // but the role is Sub regardless.
    assert_eq!(cs.bootstrap_mode(), BootstrapMode::Minimal);

    let role = captured.lock().await;
    assert_eq!(*role, Some(SessionRole::Sub));
}
