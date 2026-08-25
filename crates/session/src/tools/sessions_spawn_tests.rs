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

    async fn check_spawn_permission(
        &self,
        _parent_session_id: &str,
        _validation: &crate::spawn_validation::SpawnValidationResult,
    ) -> Result<(), crate::spawn_validation::SpawnError> {
        Ok(())
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

fn make_tool() -> SessionsSpawnTool {
    SessionsSpawnTool::new(
        Arc::new(MockSpawnValidator),
        Arc::new(MockSessionManager),
        Arc::new(MockAgentConfigLookup),
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
        prompt.contains("session creation time"),
        "budget info should indicate budget is managed at session creation time"
    );
}

#[tokio::test]
async fn test_generate_prompt_budget_always_available() {
    let tool = make_tool();
    let ctx = PromptGenerationContext {
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("sessions_spawn is available"),
        "budget status should always indicate available (budget managed at session creation time)"
    );
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

// ═══════════════════════════════════════════════════════════════════════
// Step 1.3: Budget removal regression — prompt must not reference budget field
// ═══════════════════════════════════════════════════════════════════════

/// After Step 1.2 removed effective_spawn_budget from FragmentContext and
/// PromptGenerationContext, the sessions_spawn prompt must not reference
/// budget-based filtering. It should instead state that budget is managed
/// at session creation time.
#[tokio::test]
async fn test_generate_prompt_no_budget_field_reference() {
    let tool = make_tool();
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);

    // Must NOT reference the old budget field
    assert!(
        !prompt.contains("effective_spawn_budget"),
        "prompt must not reference effective_spawn_budget, got: {prompt}"
    );
    assert!(
        !prompt.contains("spawn_budget"),
        "prompt must not reference spawn_budget, got: {prompt}"
    );

    // Must state budget is managed at session creation time
    assert!(
        prompt.contains("session creation time"),
        "prompt should indicate budget is managed at session creation time, got: {prompt}"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Step 1.4: Two-step separation tests
// ═══════════════════════════════════════════════════════════════════════

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Track which methods were called and in what order.
#[derive(Default)]
struct CallLog {
    validate_called: AtomicBool,
    check_permission_called: AtomicBool,
    create_child_called: AtomicBool,
}

/// A SpawnValidator that tracks call order and returns configurable results.
struct TrackingSpawnValidator {
    /// If `Some`, `validate_spawn` returns this error; otherwise Ok with a
    /// default SpawnValidationResult.
    validate_error: Mutex<Option<crate::spawn_validation::SpawnError>>,
    /// If `Some`, `check_spawn_permission` returns this error; otherwise Ok.
    permission_error: Mutex<Option<crate::spawn_validation::SpawnError>>,
    log: Arc<CallLog>,
}

impl TrackingSpawnValidator {
    fn new() -> Self {
        Self {
            validate_error: Mutex::new(None),
            permission_error: Mutex::new(None),
            log: Arc::new(CallLog::default()),
        }
    }

    fn with_validate_error(e: crate::spawn_validation::SpawnError) -> Self {
        Self {
            validate_error: Mutex::new(Some(e)),
            permission_error: Mutex::new(None),
            log: Arc::new(CallLog::default()),
        }
    }

    fn with_permission_error(e: crate::spawn_validation::SpawnError) -> Self {
        Self {
            validate_error: Mutex::new(None),
            permission_error: Mutex::new(Some(e)),
            log: Arc::new(CallLog::default()),
        }
    }

    fn log(&self) -> Arc<CallLog> {
        Arc::clone(&self.log)
    }
}

#[async_trait::async_trait]
impl crate::spawn_validation::SpawnValidator for TrackingSpawnValidator {
    async fn validate_spawn(
        &self,
        _parent_session_id: &str,
        _target_agent_id: Option<&str>,
    ) -> Result<crate::spawn_validation::SpawnValidationResult, crate::spawn_validation::SpawnError>
    {
        self.log.validate_called.store(true, Ordering::SeqCst);
        if let Some(e) = self.validate_error.lock().unwrap().take() {
            return Err(e);
        }
        Ok(crate::spawn_validation::SpawnValidationResult {
            config: closeclaw_config::agents::ResolvedAgentConfig {
                id: "child-agent".to_string(),
                name: "child-agent".to_string(),
                parent_id: None,
                model: Some(closeclaw_config::agents::ModelSpec::single("test-model")),
                workspace: None,
                agent_dir: None,
                bootstrap_mode: closeclaw_common::BootstrapMode::Full,
                skills: vec![],
                tools: vec![],
                disallowed_tools: vec![],
                subagents: closeclaw_config::agents::SubagentsConfig::default(),
                memory: closeclaw_config::agents::MemoryConfig::default(),
                hooks: vec![],
                parallel_tool_calls: true,
                memory_configured: false,
                source: closeclaw_config::agents::ConfigSource::User,
            },
            effective_max_spawn_depth: 1,
            spawn_timeout: Some(172800),
            timeout_warning_secs: None,
            timeout_notify_interval_ratio: None,
        })
    }

    async fn check_spawn_permission(
        &self,
        _parent_session_id: &str,
        _validation: &crate::spawn_validation::SpawnValidationResult,
    ) -> Result<(), crate::spawn_validation::SpawnError> {
        self.log
            .check_permission_called
            .store(true, Ordering::SeqCst);
        if let Some(e) = self.permission_error.lock().unwrap().take() {
            return Err(e);
        }
        Ok(())
    }
}

/// SessionManager that records whether create_child_session was called.
struct RecordingSessionManager {
    log: Arc<CallLog>,
}

#[async_trait::async_trait]
impl SessionManagerOps for RecordingSessionManager {
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
        self.log.create_child_called.store(true, Ordering::SeqCst);
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

fn make_tool_context(session_id: &str) -> closeclaw_common::tool_trait::ToolContext {
    closeclaw_common::tool_trait::ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: Some(session_id.to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
    }
}

fn make_spawn_args(task: &str) -> serde_json::Value {
    serde_json::json!({ "task": task })
}

/// Test: When `validate_spawn` fails (precondition failure),
/// `check_spawn_permission` must NOT be called.
#[tokio::test]
async fn test_two_step_precondition_failure_skips_permission() {
    let validator = TrackingSpawnValidator::with_validate_error(
        crate::spawn_validation::SpawnError::DepthExceeded { current: 1, max: 0 },
    );
    let log = validator.log();
    let sm = Arc::new(RecordingSessionManager {
        log: Arc::clone(&log),
    });
    let tool = SessionsSpawnTool::new(Arc::new(validator), sm, Arc::new(MockAgentConfigLookup));

    let ctx = make_tool_context("parent-session");
    let args = make_spawn_args("test task");

    let err = tool.call(args, &ctx).await.expect_err("should fail");
    assert!(
        matches!(
            err,
            closeclaw_common::tool_trait::ToolCallError::ExecutionFailed(_)
        ),
        "expected ExecutionFailed, got: {:?}",
        err
    );

    assert!(
        log.validate_called.load(Ordering::SeqCst),
        "validate_spawn should be called"
    );
    assert!(
        !log.check_permission_called.load(Ordering::SeqCst),
        "check_spawn_permission should NOT be called when validate_spawn fails"
    );
    assert!(
        !log.create_child_called.load(Ordering::SeqCst),
        "create_child_session should NOT be called"
    );
}

/// Test: When `validate_spawn` passes but `check_spawn_permission` denies,
/// the tool returns PermissionDenied directly — no approval flow is
/// submitted (design doc §Spawn 控制流程: Deny → return error).
#[tokio::test]
async fn test_two_step_permission_denied_returns_error() {
    let validator = TrackingSpawnValidator::with_permission_error(
        crate::spawn_validation::SpawnError::PermissionDenied {
            agent_id: "child-agent".to_string(),
            reason: "not allowed".to_string(),
        },
    );
    let log = validator.log();
    let sm = Arc::new(RecordingSessionManager {
        log: Arc::clone(&log),
    });
    let tool = SessionsSpawnTool::new(Arc::new(validator), sm, Arc::new(MockAgentConfigLookup));

    let ctx = make_tool_context("parent-session");
    let args = make_spawn_args("test task");

    let result = tool.call(args, &ctx).await;
    match result {
        Err(closeclaw_common::tool_trait::ToolCallError::PermissionDenied(reason)) => {
            assert_eq!(reason, "not allowed");
        }
        other => panic!("expected PermissionDenied, got: {:?}", other),
    }

    assert!(
        log.validate_called.load(Ordering::SeqCst),
        "validate_spawn should be called"
    );
    assert!(
        log.check_permission_called.load(Ordering::SeqCst),
        "check_spawn_permission should be called after validate_spawn"
    );
    assert!(
        !log.create_child_called.load(Ordering::SeqCst),
        "create_child_session should NOT be called when permission denied"
    );
}

/// Test: Permission denied returns the exact reason string from SpawnError,
/// ensuring error propagation fidelity.
#[tokio::test]
async fn test_permission_denied_error_message_propagated() {
    let reason_text = "agent 'secret-agent' is not in the parent allowlist";
    let validator = TrackingSpawnValidator::with_permission_error(
        crate::spawn_validation::SpawnError::PermissionDenied {
            agent_id: "secret-agent".to_string(),
            reason: reason_text.to_string(),
        },
    );
    let log = validator.log();
    let sm = Arc::new(RecordingSessionManager {
        log: Arc::clone(&log),
    });
    let tool = SessionsSpawnTool::new(Arc::new(validator), sm, Arc::new(MockAgentConfigLookup));

    let ctx = make_tool_context("parent-session");
    let args = make_spawn_args("test task");

    let err = tool.call(args, &ctx).await.expect_err("should fail");
    match err {
        closeclaw_common::tool_trait::ToolCallError::PermissionDenied(msg) => {
            assert_eq!(msg, reason_text, "error reason must be propagated verbatim");
        }
        other => panic!("expected PermissionDenied, got: {:?}", other),
    }

    assert!(
        !log.create_child_called.load(Ordering::SeqCst),
        "create_child_session must NOT be called when permission denied"
    );
}

/// Test: When both `validate_spawn` and `check_spawn_permission` pass,
/// the child session is created.
#[tokio::test]
async fn test_two_step_both_pass_creates_child() {
    let validator = TrackingSpawnValidator::new();
    let log = validator.log();
    let sm = Arc::new(RecordingSessionManager {
        log: Arc::clone(&log),
    });
    let tool = SessionsSpawnTool::new(Arc::new(validator), sm, Arc::new(MockAgentConfigLookup));

    let ctx = make_tool_context("parent-session");
    let args = make_spawn_args("test task");

    let result = tool.call(args, &ctx).await.expect("should succeed");
    assert!(
        result.data.get("session_id").is_some(),
        "result should contain session_id"
    );

    assert!(
        log.validate_called.load(Ordering::SeqCst),
        "validate_spawn should be called"
    );
    assert!(
        log.check_permission_called.load(Ordering::SeqCst),
        "check_spawn_permission should be called"
    );
    assert!(
        log.create_child_called.load(Ordering::SeqCst),
        "create_child_session should be called"
    );
}
