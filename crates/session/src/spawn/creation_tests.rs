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
}

impl MockCreationContext {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cs = ConversationSession::new(
            "parent-session".to_string(),
            "test-model".to_string(),
            tmp.path().to_path_buf(),
        );
        Self {
            parent_session: Arc::new(RwLock::new(cs)),
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
