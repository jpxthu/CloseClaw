//! Unit tests for child session creation logic.
//!
//! Covers:
//! - Task injection into child system prompt (AppendSection)
//! - Trigger message in pending queue uses "user" role (not "assistant")
//! - Task content is forwarded via system_appends, not as pending message

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
    /// Optional agent config override for `get_agent_config`.
    agent_config: Option<ResolvedAgentConfig>,
    /// Optional override for `sender_id`: None = use default "test-user",
    /// Some(None) = return None.
    sender_id_override: Option<Option<String>>,
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
            agent_config: None,
            sender_id_override: None,
        }
    }

    /// Create with an agent config override for `get_agent_config`.
    fn with_agent_config(config: ResolvedAgentConfig) -> Self {
        let mut ctx = Self::new();
        ctx.agent_config = Some(config);
        ctx
    }

    /// Create with `sender_id` returning None (force "default" user_id).
    fn with_no_sender_id() -> Self {
        let mut ctx = Self::new();
        ctx.sender_id_override = Some(None);
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
        self.agent_config.clone()
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
        match &self.sender_id_override {
            Some(override_val) => override_val.clone(),
            None => Some("test-user".to_string()),
        }
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
        memory_configured: false,
        source: closeclaw_config::agents::ConfigSource::User,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

/// Verify that the trigger message uses "user" role (not "assistant").
///
/// The task is now injected into the system prompt (AppendSection), and
/// a minimal trigger message drives the first LLM invocation. The trigger
/// message must still carry the "user" role.
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
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
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
        "trigger message must use 'user' role, got {:?}",
        msg.role
    );
}

/// Verify that task content is forwarded via system_appends and a
/// minimal trigger message is placed in the pending queue.
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
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);

    // Pending message is a minimal trigger, not the task text.
    assert_eq!(
        pending[0].content, "Begin your assigned task.",
        "pending message must be the trigger text"
    );

    // Task content lives in system appends.
    let appends = cs.system_appends();
    assert!(
        appends
            .iter()
            .any(|s| s == &"## Task\nRun unit tests and report results"),
        "task text must be in system_appends, got: {:?}",
        appends
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
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
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

/// Verify that trigger message role is "user" even with different spawn modes.
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
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
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
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
    }
}

/// **Test 1 — Whitelist injection生效**: When agent config has a non-empty
/// skills subset, the child session must have `agent_skills == Some(whitelist)`.
#[tokio::test]
async fn test_skills_whitelist_injected() {
    let mut config = make_config("child-agent");
    config.skills = vec!["skill-a".into(), "skill-b".into()];
    let ctx = MockCreationContext::with_agent_config(config);
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
    let ctx = MockCreationContext::with_agent_config(config);
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
    let ctx = MockCreationContext::with_agent_config(config);
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
    let ctx = MockCreationContext::with_agent_config(config);
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
    let ctx = MockCreationContext::with_agent_config(config);
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
    let ctx = MockCreationContext::new();
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
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
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

    // Pending message is a trigger, not the task text.
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "Begin your assigned task.");

    // Task content is in system_appends, not in the pending message.
    let appends = cs.system_appends();
    assert!(
        appends.iter().any(|s| s.contains("Analyze the codebase")),
        "task text must be in system_appends"
    );
    assert!(
        !pending[0].content.contains("## Custom Template"),
        "pending trigger must NOT contain the template prefix"
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
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);
    // Pending is the trigger, not the task text.
    assert_eq!(pending[0].content, "Begin your assigned task.");
    // Task text lives in system_appends.
    let appends = cs.system_appends();
    assert!(
        appends.iter().any(|s| s.contains(task_text)),
        "task text must be in system_appends, got: {:?}",
        appends
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
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "Begin your assigned task.");
    // Task content is in system_appends.
    let appends = cs.system_appends();
    assert!(
        appends.iter().any(|s| s.contains("Simple task")),
        "task text must be in system_appends"
    );
    // System prompt should not contain any template-related text
    let sys_prompt = cs.system_prompt().map(|s| s.to_owned()).unwrap_or_default();
    assert!(
        !sys_prompt.contains("Template prefix"),
        "system prompt should not contain template text when prefix is None"
    );
}

// ── Workspace fallback chain tests ──────────────────────────────────────

/// Level 3 fallback: when no explicit workspace or config.workspace,
/// the dedicated workspace directory is used.
#[tokio::test]
async fn test_level3_dedicated_workspace_fallback() {
    let ctx = MockCreationContext::new();
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
        "Level 3 fallback must produce config_dir/workspaces/child-agent/test-user/"
    );
}

/// Level 3 fallback path is compatible with `is_workspace_path()` authorization.
#[tokio::test]
async fn test_level3_dedicated_path_matches_workspace_authorization() {
    let ctx = MockCreationContext::new();
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

/// Level 3 fallback uses user_id from `sender_id()`.
#[tokio::test]
async fn test_level3_dedicated_uses_sender_id() {
    let ctx = MockCreationContext::new();
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
        "Level 3 fallback must include sender_id as the user_id component"
    );
}

// ── Edge cases for workspace fallback chain ────────────────────────────

/// Empty string workspace is treated as Level 1 (explicit), same as a
/// non-empty string. `PathBuf::from("")` is valid and returned as-is.
#[tokio::test]
async fn test_empty_string_workspace_treated_as_explicit() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        workspace: Some(""),
        ..default_params()
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    assert_eq!(
        result.workspace_path,
        std::path::PathBuf::from(""),
        "empty string workspace should be returned as-is (Level 1 explicit)"
    );
}

/// When `sender_id()` returns None, Level 3 fallback uses "default"
/// as the user_id component.
#[tokio::test]
async fn test_level3_sender_id_none_uses_default() {
    // Use a context where sender_id returns None.
    let ctx = MockCreationContext::with_no_sender_id();
    let config = make_config("child-agent");
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    let expected = ctx
        .config_dir()
        .join("workspaces")
        .join("child-agent")
        .join("default");
    assert_eq!(
        result.workspace_path, expected,
        "Level 3 fallback must use 'default' when sender_id is None"
    );
}

/// Verify the 3-level fallback chain in priority order:
/// explicit > config.workspace > dedicated directory.
/// When both explicit and config.workspace are set, explicit wins.
#[tokio::test]
async fn test_fallback_chain_explicit_beats_config() {
    let ctx = MockCreationContext::new();
    let mut config = make_config("child-agent");
    config.workspace = Some(std::path::PathBuf::from("/config/ws"));
    let params = ChildSessionCreationParams {
        workspace: Some("/explicit/ws"),
        ..default_params()
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    assert_eq!(
        result.workspace_path,
        std::path::PathBuf::from("/explicit/ws"),
        "explicit workspace must take priority over config.workspace"
    );
}

/// When config.workspace is Some and no explicit workspace is provided,
/// config.workspace wins over the Level 3 dedicated directory.
#[tokio::test]
async fn test_fallback_chain_config_beats_dedicated() {
    let ctx = MockCreationContext::new();
    let mut config = make_config("child-agent");
    config.workspace = Some(std::path::PathBuf::from("/config/ws"));
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    assert_eq!(
        result.workspace_path,
        std::path::PathBuf::from("/config/ws"),
        "config.workspace must win over Level 3 dedicated directory"
    );
}

// ── Fork mode: task in system prompt, parent history in messages ────────

/// Helper: extract text content from a SessionMessage's content blocks.
fn msg_text(msg: &crate::llm_session::SessionMessage) -> String {
    use closeclaw_common::ContentBlock;
    msg.content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Helper: create a MockCreationContext with pre-populated parent history.
async fn ctx_with_parent_history() -> MockCreationContext {
    use closeclaw_common::ContentBlock;
    let ctx = MockCreationContext::new();
    let parent = ctx
        .get_parent_conversation_session("parent-session")
        .await
        .unwrap();
    let mut guard = parent.write().await;
    guard.push_message("user", vec![ContentBlock::Text("Hello parent".to_string())]);
    guard.push_message(
        "assistant",
        vec![ContentBlock::Text("Hi there!".to_string())],
    );
    drop(guard);
    ctx
}

/// Verify fork mode: task injected into system_appends, parent conversation
/// history cloned into messages, and trigger message in pending queue.
///
/// Design doc §Fork mode: "fork 模式下 task 始终位于 system prompt，
/// 不依赖对话消息顺序" — task in system_appends (part of system prompt),
/// parent history in messages, trigger message is minimal.
#[tokio::test]
async fn test_fork_mode_task_in_system_appends_parent_history_in_messages() {
    let ctx = ctx_with_parent_history().await;
    let config = make_config("child-agent");
    let params = ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 1,
        task: "Fork task description",
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: true,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: None,
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
    };

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("create_child_conversation_session should succeed");

    let cs = result.conversation_session.read().await;

    // 1. Task lives in system_appends, NOT in pending messages.
    let appends = cs.system_appends();
    assert!(
        appends
            .iter()
            .any(|s| s.contains("## Task\nFork task description")),
        "task must be in system_appends in fork mode, got: {:?}",
        appends
    );

    // 2. Pending queue has exactly one trigger message (not the task).
    let pending = cs.get_pending_messages();
    assert_eq!(
        pending.len(),
        1,
        "pending must have exactly 1 trigger message"
    );
    assert_eq!(pending[0].content, "Begin your assigned task.");
    assert!(
        !pending[0].content.contains("Fork task description"),
        "trigger message must not contain the task text"
    );

    // 3. Parent conversation history is present in messages.
    let messages = &cs.messages;
    let texts: Vec<String> = messages.iter().map(msg_text).collect();
    assert!(
        texts.iter().any(|t| t == "Hello parent"),
        "fork must clone parent history into messages, got: {:?}",
        texts
    );
    assert!(
        texts.iter().any(|t| t == "Hi there!"),
        "fork must clone parent history (assistant reply too)"
    );

    // 4. Task is NOT in the conversation messages (it's in system_appends).
    assert!(
        !texts.iter().any(|t| t.contains("Fork task description")),
        "task text must NOT be in conversation messages in fork mode"
    );
}
