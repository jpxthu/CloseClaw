//! Unit tests for child session creation logic.
//!
//! Covers:
//! - Task injection uses "user" role (not "assistant")
//! - Task content is correctly forwarded as pending message

use std::sync::Arc;

use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::ResolvedAgentConfig;
use tokio::sync::RwLock;

use super::context::SpawnCreationContext;
use super::creation::{create_child_conversation_session, ChildSessionCreationParams};
use super::types::SpawnMode;
use crate::llm_session::ConversationSession;
use crate::persistence::{ReasoningLevel, SessionCheckpoint};

// ── Mock implementation ────────────────────────────────────────────────

/// Minimal mock of [`SpawnCreationContext`] for unit tests.
///
/// Provides just enough to let `create_child_conversation_session` succeed
/// without touching the gateway or LLM layer.
struct MockCreationContext {
    /// Parent conversation session used for token derivation and fork.
    parent_session: Arc<RwLock<ConversationSession>>,
    /// Mock config directory path.
    config_dir: std::path::PathBuf,
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
        }
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

    fn system_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::SystemPromptBuilder>> {
        None
    }

    fn prompt_overrides(&self) -> Option<closeclaw_common::PromptOverrides> {
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

    async fn parent_workspace(&self, _parent_session_id: &str) -> Option<std::path::PathBuf> {
        let guard = self.parent_session.read().await;
        Some(guard.workdir().to_path_buf())
    }

    fn config_dir(&self) -> &std::path::Path {
        &self.config_dir
    }
}

/// Build a minimal [`ResolvedAgentConfig`] for testing.
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
        source: closeclaw_config::agents::ConfigSource::User,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

/// Verify that task injection uses "user" role (not "assistant").
///
/// This is the primary invariant from the design doc: the task is injected
/// as the first *user* message in the child session's transcript.
#[tokio::test]
async fn test_task_injected_with_user_role() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 0,
        task: "Analyze the codebase",
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: false,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: None,
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1, "should have exactly one pending message");

    let msg = &pending[0];
    assert_eq!(
        msg.role.as_deref(),
        Some("user"),
        "task must be injected with 'user' role, got {:?}",
        msg.role
    );
}

/// Verify that task content is correctly forwarded in the pending message.
#[tokio::test]
async fn test_task_content_forwarded() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 0,
        task: "Run unit tests and report results",
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: false,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: None,
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);

    let msg = &pending[0];
    assert_eq!(
        msg.content, "Run unit tests and report results",
        "task content must match exactly"
    );
}

/// Verify that the pending message ID follows the expected pattern.
#[tokio::test]
async fn test_pending_message_id_format() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
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
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);

    let msg = &pending[0];
    // The message ID should be "<child_session_id>-task"
    assert!(
        msg.message_id.ends_with("-task"),
        "message ID should end with '-task', got: {}",
        msg.message_id
    );
    assert_eq!(
        msg.message_id,
        format!("{}-task", result.session_id),
        "message ID should be <child_session_id>-task"
    );
}

/// Verify that task role is "user" even with different spawn modes.
#[tokio::test]
async fn test_task_role_user_in_session_mode() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 0,
        task: "Persistent session task",
        light_context: false,
        workspace: None,
        mode: SpawnMode::Session,
        fork: false,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: None,
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].role.as_deref(),
        Some("user"),
        "task role must be 'user' in Session mode too"
    );
}

// ── Mock: no parent workspace ──────────────────────────────────────────

/// Mock that returns `None` from `parent_workspace`, simulating a parent
/// session that has no workspace directory set. Used to test the
/// Level 4 (dedicated directory) fallback path.
struct MockCreationContextWithNoParentWorkspace {
    inner: MockCreationContext,
}

impl MockCreationContextWithNoParentWorkspace {
    fn new() -> Self {
        Self {
            inner: MockCreationContext::new(),
        }
    }
}

#[async_trait::async_trait]
impl SpawnCreationContext for MockCreationContextWithNoParentWorkspace {
    async fn get_parent_conversation_session(
        &self,
        parent_session_id: &str,
    ) -> Option<Arc<RwLock<ConversationSession>>> {
        self.inner
            .get_parent_conversation_session(parent_session_id)
            .await
    }

    async fn load_checkpoint(&self, session_id: &str) -> Option<SessionCheckpoint> {
        self.inner.load_checkpoint(session_id).await
    }

    async fn save_checkpoint(&self, cp: &SessionCheckpoint) {
        self.inner.save_checkpoint(cp).await
    }

    fn get_agent_config(&self, agent_id: &str) -> Option<ResolvedAgentConfig> {
        self.inner.get_agent_config(agent_id)
    }

    fn shutdown_signal(&self) -> Option<Arc<dyn closeclaw_common::ShutdownSignal>> {
        self.inner.shutdown_signal()
    }

    fn default_reasoning_level(&self) -> ReasoningLevel {
        self.inner.default_reasoning_level()
    }

    fn llm_caller(&self) -> Option<Arc<dyn closeclaw_common::LlmCaller>> {
        self.inner.llm_caller()
    }

    fn system_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::SystemPromptBuilder>> {
        self.inner.system_prompt_builder()
    }

    fn prompt_overrides(&self) -> Option<closeclaw_common::PromptOverrides> {
        self.inner.prompt_overrides()
    }

    fn dynamic_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::DynamicPromptBuilder>> {
        self.inner.dynamic_prompt_builder()
    }

    fn skill_listing_provider(&self) -> Option<Arc<dyn closeclaw_common::SkillListingProvider>> {
        self.inner.skill_listing_provider()
    }

    async fn sender_id(&self, session_id: &str) -> Option<String> {
        self.inner.sender_id(session_id).await
    }

    async fn parent_workspace(&self, _parent_session_id: &str) -> Option<std::path::PathBuf> {
        None // Force fallback to Level 4 (dedicated directory)
    }

    fn config_dir(&self) -> &std::path::Path {
        self.inner.config_dir()
    }
}

// ── Step 1.2: Agent skills whitelist injection ────────────────────────────

/// Build a [`MockCreationContext`] that returns a [`ResolvedAgentConfig`]
/// with the given skills whitelist from `get_agent_config`.
struct MockCreationContextWithSkills {
    inner: MockCreationContext,
    agent_config: Option<ResolvedAgentConfig>,
}

impl MockCreationContextWithSkills {
    fn new(agent_config: Option<ResolvedAgentConfig>) -> Self {
        Self {
            inner: MockCreationContext::new(),
            agent_config,
        }
    }
}

#[async_trait::async_trait]
impl SpawnCreationContext for MockCreationContextWithSkills {
    async fn get_parent_conversation_session(
        &self,
        parent_session_id: &str,
    ) -> Option<Arc<RwLock<ConversationSession>>> {
        self.inner
            .get_parent_conversation_session(parent_session_id)
            .await
    }

    async fn load_checkpoint(&self, session_id: &str) -> Option<SessionCheckpoint> {
        self.inner.load_checkpoint(session_id).await
    }

    async fn save_checkpoint(&self, cp: &SessionCheckpoint) {
        self.inner.save_checkpoint(cp).await
    }

    fn get_agent_config(&self, _agent_id: &str) -> Option<ResolvedAgentConfig> {
        self.agent_config.clone()
    }

    fn shutdown_signal(&self) -> Option<Arc<dyn closeclaw_common::ShutdownSignal>> {
        self.inner.shutdown_signal()
    }

    fn default_reasoning_level(&self) -> ReasoningLevel {
        self.inner.default_reasoning_level()
    }

    fn llm_caller(&self) -> Option<Arc<dyn closeclaw_common::LlmCaller>> {
        self.inner.llm_caller()
    }

    fn system_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::SystemPromptBuilder>> {
        self.inner.system_prompt_builder()
    }

    fn prompt_overrides(&self) -> Option<closeclaw_common::PromptOverrides> {
        self.inner.prompt_overrides()
    }

    fn dynamic_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::DynamicPromptBuilder>> {
        self.inner.dynamic_prompt_builder()
    }

    fn skill_listing_provider(&self) -> Option<Arc<dyn closeclaw_common::SkillListingProvider>> {
        self.inner.skill_listing_provider()
    }

    async fn sender_id(&self, session_id: &str) -> Option<String> {
        self.inner.sender_id(session_id).await
    }

    async fn parent_workspace(&self, parent_session_id: &str) -> Option<std::path::PathBuf> {
        self.inner.parent_workspace(parent_session_id).await
    }

    fn config_dir(&self) -> &std::path::Path {
        self.inner.config_dir()
    }
}

/// Build [`ChildSessionCreationParams`] with defaults suitable for skills tests.
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
    }
}

/// **Test 1 — Whitelist injection生效**: When agent config has a non-empty
/// skills subset, the child session must have `agent_skills == Some(whitelist)`.
#[tokio::test]
async fn test_skills_whitelist_injected() {
    let mut config = make_config("child-agent");
    config.skills = vec!["skill-a".into(), "skill-b".into()];
    let ctx = MockCreationContextWithSkills::new(Some(config));
    let params = default_params();

    let result = create_child_conversation_session(
        &ctx,
        &ctx.get_agent_config("child-agent").unwrap(),
        &params,
    )
    .await
    .expect("should succeed");

    let cs = result.conversation_session.read().await;
    let skills = cs.agent_skills().expect("agent_skills should be Some");
    assert_eq!(
        skills,
        &["skill-a".to_string(), "skill-b".to_string(),],
        "whitelist must match config.effective_skills()"
    );
}

/// **Test 2 — Wildcard semantics**: Empty or `["*"]` skills must not be
/// injected (agent_skills stays None), matching resolve.rs behavior.
#[tokio::test]
async fn test_skills_wildcard_empty_no_injection() {
    let mut config = make_config("child-agent");
    config.skills = vec![]; // empty = wildcard
    let ctx = MockCreationContextWithSkills::new(Some(config));
    let params = default_params();

    let result = create_child_conversation_session(
        &ctx,
        &ctx.get_agent_config("child-agent").unwrap(),
        &params,
    )
    .await
    .expect("should succeed");

    let cs = result.conversation_session.read().await;
    assert!(
        cs.agent_skills().is_none(),
        "empty skills must not inject whitelist"
    );
}

/// Wildcard `["*"]` must also leave agent_skills as None.
#[tokio::test]
async fn test_skills_wildcard_star_no_injection() {
    let mut config = make_config("child-agent");
    config.skills = vec!["*".into()];
    let ctx = MockCreationContextWithSkills::new(Some(config));
    let params = default_params();

    let result = create_child_conversation_session(
        &ctx,
        &ctx.get_agent_config("child-agent").unwrap(),
        &params,
    )
    .await
    .expect("should succeed");

    let cs = result.conversation_session.read().await;
    assert!(
        cs.agent_skills().is_none(),
        "[\"*\"] skills must not inject whitelist"
    );
}

/// **Test 3 — Scenario independence**: Fork mode + lightContext both inject whitelist.
#[tokio::test]
async fn test_skills_injected_in_fork_mode() {
    let mut config = make_config("child-agent");
    config.skills = vec!["only-this".into()];
    let ctx = MockCreationContextWithSkills::new(Some(config));
    let params = ChildSessionCreationParams {
        fork: true,
        light_context: false,
        ..default_params()
    };

    let result = create_child_conversation_session(
        &ctx,
        &ctx.get_agent_config("child-agent").unwrap(),
        &params,
    )
    .await
    .expect("should succeed");

    let cs = result.conversation_session.read().await;
    let skills = cs
        .agent_skills()
        .expect("agent_skills should be Some in fork mode");
    assert_eq!(skills, &["only-this".to_string()]);
}

/// lightContext with non-wildcard skills still injects whitelist.
#[tokio::test]
async fn test_skills_injected_in_light_context() {
    let mut config = make_config("child-agent");
    config.skills = vec!["light-skill".into()];
    let ctx = MockCreationContextWithSkills::new(Some(config));
    let params = ChildSessionCreationParams {
        light_context: true,
        fork: false,
        ..default_params()
    };

    let result = create_child_conversation_session(
        &ctx,
        &ctx.get_agent_config("child-agent").unwrap(),
        &params,
    )
    .await
    .expect("should succeed");

    let cs = result.conversation_session.read().await;
    let skills = cs
        .agent_skills()
        .expect("agent_skills should be Some in light context");
    assert_eq!(skills, &["light-skill".to_string()]);
}

/// **Test 4 — No config boundary**: When get_agent_config returns None, the
/// child session creation must not panic and agent_skills stays None.
#[tokio::test]
async fn test_skills_no_config_no_panic() {
    let ctx = MockCreationContextWithSkills::new(None);
    let params = default_params();

    let unknown_config = make_config("unknown-agent");
    let result = create_child_conversation_session(&ctx, &unknown_config, &params)
        .await
        .expect("should not panic even without agent config");

    let cs = result.conversation_session.read().await;
    assert!(
        cs.agent_skills().is_none(),
        "no config should result in no whitelist injection"
    );
}

// ── Gap 4: Prompt template injection into system prompt ───────────────────

/// Verify prompt_template_prefix is injected into system prompt, not the user message.
#[tokio::test]
async fn test_prompt_template_injected_into_system_prompt() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 0,
        task: "Analyze the codebase",
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: false,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: Some("## Custom Template\nRead only."),
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;

    // System prompt should contain the template text
    let sys_prompt = cs.system_prompt().map(|s| s.to_owned()).unwrap_or_default();
    assert!(
        sys_prompt.contains("## Custom Template"),
        "system prompt should contain the template text"
    );
    assert!(
        sys_prompt.contains("Read only."),
        "system prompt should contain the template body"
    );

    // User message (task) should NOT contain the template text
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "Analyze the codebase");
    assert!(
        !pending[0].content.contains("## Custom Template"),
        "user message must NOT contain the template prefix"
    );
}

/// Verify task content is unchanged when prompt_template_prefix is provided.
#[tokio::test]
async fn test_task_unchanged_with_prompt_template() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let task_text = "Run tests and report results";
    let params = ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 0,
        task: task_text,
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: false,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: Some("Template prefix"),
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].content, task_text,
        "task content must be exactly the original task text"
    );
}

/// Verify behavior without prompt_template_prefix is unchanged.
#[tokio::test]
async fn test_no_prompt_template_unchanged_behavior() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 0,
        task: "Simple task",
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: false,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: None,
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "Simple task");
    // System prompt should not contain any template-related text
    let sys_prompt = cs.system_prompt().map(|s| s.to_owned()).unwrap_or_default();
    assert!(
        !sys_prompt.contains("Template prefix"),
        "system prompt should not contain template text when prefix is None"
    );
}

// ── Workspace fallback chain tests ──────────────────────────────────────

/// Level 3 fallback: when no explicit workspace or config.workspace,
/// the parent session workspace is used.
#[tokio::test]
async fn test_level3_parent_workspace_fallback() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    // MockCreationContext::parent_workspace returns the parent session's workdir
    let parent_ws = ctx.parent_workspace("parent-session").await.unwrap();
    assert_eq!(
        result.workspace_path, parent_ws,
        "Level 3 fallback must use parent session workspace"
    );
}

/// Level 4 fallback: when parent_workspace returns None and parent session
/// exists, the dedicated workspace directory is used.
#[tokio::test]
async fn test_level4_dedicated_workspace_fallback() {
    let ctx = MockCreationContextWithNoParentWorkspace::new();
    let config = make_config("child-agent");
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    let expected = ctx
        .config_dir()
        .join("workspaces")
        .join("child-agent")
        .join("test-user");
    assert_eq!(
        result.workspace_path, expected,
        "Level 4 fallback must produce config_dir/workspaces/child-agent/test-user/"
    );
}

/// Level 4 fallback path is compatible with `is_workspace_path()` authorization.
#[tokio::test]
async fn test_level4_dedicated_path_matches_workspace_authorization() {
    let ctx = MockCreationContextWithNoParentWorkspace::new();
    let config = make_config("my-agent");
    let params = ChildSessionCreationParams {
        task: "workspace auth test",
        ..default_params()
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    let ws = &result.workspace_path;
    let ws_str = ws.to_string_lossy();

    assert!(
        ws_str.contains("/workspaces/"),
        "workspace path must contain '/workspaces/' segment: {}",
        ws_str
    );
    assert!(
        ws_str.ends_with("/my-agent/test-user"),
        "workspace path must end with agent_id/user_id: {}",
        ws_str
    );
    let config_dir = ctx.config_dir();
    assert!(
        ws.starts_with(config_dir),
        "workspace path must be under config_dir: {} vs {}",
        ws.display(),
        config_dir.display()
    );
}

/// Explicit workspace argument takes highest priority.
#[tokio::test]
async fn test_explicit_workspace_overrides_fallback() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        workspace: Some("/custom/explicit/workspace"),
        ..default_params()
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    assert_eq!(
        result.workspace_path,
        std::path::PathBuf::from("/custom/explicit/workspace"),
        "explicit workspace must override all fallbacks"
    );
}

/// Config-level workspace takes priority over parent workspace and dedicated dir.
#[tokio::test]
async fn test_config_workspace_overrides_fallback() {
    let ctx = MockCreationContext::new();
    let mut config = make_config("child-agent");
    config.workspace = Some(std::path::PathBuf::from("/config/specified/workspace"));
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    assert_eq!(
        result.workspace_path,
        std::path::PathBuf::from("/config/specified/workspace"),
        "config.workspace must override parent workspace and dedicated dir"
    );
}

/// Level 4 fallback uses user_id from `sender_id()`.
#[tokio::test]
async fn test_level4_dedicated_uses_sender_id() {
    let ctx = MockCreationContextWithNoParentWorkspace::new();
    let config = make_config("test-agent");
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    // MockCreationContext::sender_id returns "test-user"
    let expected = ctx
        .config_dir()
        .join("workspaces")
        .join("test-agent")
        .join("test-user");
    assert_eq!(
        result.workspace_path, expected,
        "Level 4 fallback must include sender_id as the user_id component"
    );
}
