//! Tests for [`SessionsSpawnTool`] prompt generation (Step 1.3).
//!
//! Covers:
//! - Budget awareness: available vs exhausted vs unknown budget
//! - Combination suggestions: sessions_yield, sessions_steer, sessions_kill
//! - Workdir guidance: workspace path inherited by children
//! - When-to-use and usage principles always present

use super::sessions_spawn::SessionsSpawnTool;
use super::SessionManagerOps;
use closeclaw_common::tool_trait::{PromptGenerationContext, Tool, WorkdirContext};

use std::sync::Arc;

// ---------------------------------------------------------------------------
// Mock SessionManagerOps (minimal, only needed for constructor)
// ---------------------------------------------------------------------------

struct MockSessionManager;

#[async_trait::async_trait]
impl SessionManagerOps for MockSessionManager {
    async fn create_child_session(
        &self,
        _config: &closeclaw_config::agents::ResolvedAgentConfig,
        _parent_session_id: &str,
        _depth: u32,
        _task: &str,
        _light_context: bool,
        _workspace: Option<&str>,
        _mode: crate::spawn::SpawnMode,
        _fork: bool,
        _allowed_tools: Option<Vec<String>>,
        _model_override: Option<&str>,
        _parent_subagents_model: Option<&str>,
        _max_spawn_depth: u32,
        _spawn_timeout: Option<u64>,
        _label: Option<&str>,
        _prompt_template_prefix: Option<&str>,
        _timeout_warning_secs: Option<u64>,
        _timeout_notify_interval_ratio: Option<f64>,
    ) -> Result<String, String> {
        Ok("child-session-id".into())
    }

    async fn validate_child_ownership(
        &self,
        _parent_id: &str,
        _child_id: &str,
    ) -> Option<crate::spawn::ChildSessionInfo> {
        None
    }

    async fn steer_child(&self, _child_id: &str, _task: &str) -> Result<(), String> {
        Ok(())
    }

    async fn kill_child(&self, _parent_id: &str, _child_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
        None
    }

    async fn get_session_depth(&self, _session_id: &str) -> Option<u32> {
        Some(0)
    }

    async fn list_children(&self, _parent_id: &str) -> Vec<crate::spawn::ChildSessionInfo> {
        vec![]
    }

    async fn start_yield_timeout(
        self: Arc<Self>,
        _session_id: &str,
        _agent_id: &str,
        _overall_timeout_secs: u64,
        _timeout_warning_secs: Option<u64>,
        _notify_interval_ratio: Option<f64>,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Mock SpawnValidator
// ---------------------------------------------------------------------------

struct MockSpawnValidator;

#[async_trait::async_trait]
impl crate::spawn_validation::SpawnValidator for MockSpawnValidator {
    async fn validate_spawn(
        &self,
        _parent_session_id: &str,
        _target_agent_id: Option<&str>,
    ) -> Result<crate::spawn_validation::SpawnValidationResult, crate::spawn_validation::SpawnError>
    {
        Err(crate::spawn_validation::SpawnError::AgentIdRequired)
    }
}

// ---------------------------------------------------------------------------
// Mock AgentConfigLookup
// ---------------------------------------------------------------------------

struct MockAgentConfigLookup;

#[async_trait::async_trait]
impl closeclaw_agent::AgentConfigLookup for MockAgentConfigLookup {
    async fn lookup_agent_config(
        &self,
        _agent_id: &str,
    ) -> Option<closeclaw_agent::AgentConfigInfo> {
        None
    }
}

// ---------------------------------------------------------------------------
// Mock ApprovalSubmission
// ---------------------------------------------------------------------------

fn mock_approval_flow() -> closeclaw_common::permission_types::SharedApprovalSubmission {
    use closeclaw_common::permission_types::ApprovalSubmission;
    struct AutoApproveApproval;
    impl ApprovalSubmission for AutoApproveApproval {
        fn submit_inter_agent_denial(
            &self,
            _caller: &closeclaw_common::permission_types::CallerInfo,
            _from: &str,
            _to: &str,
            _risk_level: closeclaw_common::permission_types::RiskLevel,
            _session_id: &str,
            _is_sub_agent: bool,
        ) -> Option<String> {
            Some("mock-approval-id".to_string())
        }
    }
    Arc::new(tokio::sync::Mutex::new(AutoApproveApproval))
}

fn make_tool() -> SessionsSpawnTool {
    SessionsSpawnTool::new(
        Arc::new(MockSpawnValidator),
        Arc::new(MockSessionManager),
        Arc::new(MockAgentConfigLookup),
        mock_approval_flow(),
    )
}

// ---------------------------------------------------------------------------
// Budget awareness tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_generate_prompt_empty_context() {
    let tool = make_tool();
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("sessions_spawn is available"),
        "empty context should show sessions_spawn as available"
    );
    assert!(
        prompt.contains("budget is unknown"),
        "no budget info should indicate budget is unknown"
    );
}

#[tokio::test]
async fn test_generate_prompt_budget_exhausted() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        effective_spawn_budget: Some(0),
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("not available"),
        "budget 0 should indicate not available"
    );
    assert!(
        prompt.contains("exhausted"),
        "budget 0 should mention exhausted"
    );
}

#[tokio::test]
async fn test_generate_prompt_budget_available() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        effective_spawn_budget: Some(5),
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("sessions_spawn is available"),
        "budget 5 should show available"
    );
    assert!(prompt.contains("5"), "should mention the budget number");
}

// ---------------------------------------------------------------------------
// Combination suggestions tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_generate_prompt_combination_yield() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        available_tool_names: vec!["sessions_spawn".into(), "sessions_yield".into()],
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("sessions_yield"),
        "should suggest sessions_yield"
    );
    assert!(
        prompt.contains("wait for child completion"),
        "should describe yield purpose"
    );
}

#[tokio::test]
async fn test_generate_prompt_combination_all() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        available_tool_names: vec![
            "sessions_spawn".into(),
            "sessions_yield".into(),
            "sessions_steer".into(),
            "sessions_kill".into(),
        ],
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(prompt.contains("sessions_yield"));
    assert!(prompt.contains("sessions_steer"));
    assert!(prompt.contains("sessions_kill"));
    assert!(
        prompt.contains("Lifecycle management"),
        "should have lifecycle management section"
    );
}

#[tokio::test]
async fn test_generate_prompt_no_combination_when_unavailable() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        available_tool_names: vec!["sessions_spawn".into()],
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        !prompt.contains("Lifecycle management"),
        "should not suggest lifecycle tools when they are not available"
    );
}

// ---------------------------------------------------------------------------
// Workdir guidance tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_generate_prompt_workdir_inherited() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        workdir: Some(WorkdirContext {
            path: "/home/user/project".into(),
            has_git: true,
            branch: Some("main".into()),
            recent_changes: 0,
        }),
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("/home/user/project"),
        "should contain the workspace path"
    );
    assert!(
        prompt.contains("inherit"),
        "should mention workspace inheritance"
    );
}

#[tokio::test]
async fn test_generate_prompt_no_workdir_guidance() {
    let tool = make_tool();
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        !prompt.contains("Current workspace"),
        "no workdir should not show workspace guidance"
    );
}

// ---------------------------------------------------------------------------
// Always-present sections tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_generate_prompt_always_has_when_to_use() {
    let tool = make_tool();
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("parallel task execution"),
        "should describe when to use sessions_spawn"
    );
}

#[tokio::test]
async fn test_generate_prompt_always_has_usage_principles() {
    let tool = make_tool();
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("timeout"),
        "should mention timeout in usage principles"
    );
    assert!(
        prompt.contains("mode="),
        "should mention mode in usage principles"
    );
}

// ---------------------------------------------------------------------------
// Task authoring guidance tests (Gap 2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_generate_prompt_task_authoring_guidance_present() {
    let tool = make_tool();
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);

    // Must contain all three guidelines from design doc §子 Agent 提示词工程
    // 1. Brief like a colleague — say what to do and why
    assert!(
        prompt.contains("colleague"),
        "task authoring guidance must mention 'colleague' (brief like a colleague)"
    );
    assert!(
        prompt.contains("what to do and why"),
        "task authoring guidance must contain 'what to do and why'"
    );

    // 2. Don't delegate synthesis/judgment
    assert!(
        prompt.contains("don't delegate synthesis") || prompt.contains("Don't delegate synthesis"),
        "task authoring guidance must contain 'don't delegate synthesis'"
    );

    // 3. fork=true for context, plain spawn for independent subtasks
    assert!(
        prompt.contains("fork=true") || prompt.contains("fork"),
        "task authoring guidance must mention fork"
    );
}

#[tokio::test]
async fn test_generate_prompt_task_authoring_includes_all_three_strategies() {
    let tool = make_tool();
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);

    // Verify the guidance section exists as a distinct block
    assert!(
        prompt.contains("Task authoring guidelines"),
        "prompt must contain 'Task authoring guidelines' heading"
    );

    // Each guideline is a separate bullet point
    let lines: Vec<&str> = prompt.lines().collect();
    let task_guidance_lines: Vec<&str> = lines
        .iter()
        .filter(|l| {
            l.starts_with("- ")
                && (l.contains("colleague")
                    || l.contains("delegate synthesis")
                    || l.contains("fork"))
        })
        .copied()
        .collect();
    assert!(
        task_guidance_lines.len() >= 3,
        "must have at least 3 bullet points for task authoring guidance, found {}",
        task_guidance_lines.len()
    );
}
