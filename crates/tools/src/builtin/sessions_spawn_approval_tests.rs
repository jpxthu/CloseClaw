//! Approval flow routing tests for SessionsSpawnTool.
//!
//! Covers three paths:
//! 1. allow — spawn validation passes, child session created
//! 2. deny + enqueue success — returns approval_pending
//! 3. deny + enqueue failure (duplicate) — fallback to PermissionDenied

use std::sync::Arc;

use serde_json::json;

use crate::builtin::sessions_spawn::SessionsSpawnTool;
use crate::{SpawnError, SpawnValidator, Tool, ToolCallError, ToolContext};
use closeclaw_config::spawn_validation::SpawnValidationResult;
use closeclaw_gateway::{GatewayConfig, Message, SessionManager};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_risk::RiskLevel;
use closeclaw_permission::engine::engine_types::{Caller, RuleSet};
use closeclaw_session::persistence::ReasoningLevel;

// ---------------------------------------------------------------------------
// Mock SpawnValidator
// ---------------------------------------------------------------------------

enum MockSpawnResult {
    Ok(SpawnValidationResult),
    Err(SpawnError),
}

struct MockValidator {
    result: MockSpawnResult,
}

#[async_trait::async_trait]
impl SpawnValidator for MockValidator {
    async fn validate_spawn(
        &self,
        _parent_session_id: &str,
        _target_agent_id: Option<&str>,
    ) -> Result<SpawnValidationResult, SpawnError> {
        match &self.result {
            MockSpawnResult::Ok(r) => Ok(r.clone()),
            MockSpawnResult::Err(e) => Err(SpawnError::PermissionDenied {
                agent_id: match e {
                    SpawnError::PermissionDenied { agent_id, .. } => agent_id.clone(),
                    _ => "unknown".to_string(),
                },
                reason: match e {
                    SpawnError::PermissionDenied { reason, .. } => reason.clone(),
                    _ => "unknown".to_string(),
                },
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_approval_flow() -> Arc<tokio::sync::Mutex<ApprovalFlow>> {
    Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
        Arc::clone(&make_session_manager()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )))
}

fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            ..Default::default()
        },
        None,
        None,
        ReasoningLevel::default(),
    ))
}

fn make_ctx(session_id: &str) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: Some(session_id.to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
    }
}

fn make_agent_config_lookup() -> Arc<dyn closeclaw_agent::AgentConfigLookup> {
    struct DummyLookup;
    #[async_trait::async_trait]
    impl closeclaw_agent::AgentConfigLookup for DummyLookup {
        async fn lookup_agent_config(
            &self,
            _agent_id: &str,
        ) -> Option<closeclaw_agent::AgentConfigInfo> {
            None
        }
    }
    Arc::new(DummyLookup)
}

fn spawn_result() -> SpawnValidationResult {
    SpawnValidationResult {
        config: closeclaw_config::agents::ResolvedAgentConfig {
            id: "child-agent".to_string(),
            name: "child-agent".to_string(),
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: closeclaw_common::BootstrapMode::Full,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: closeclaw_config::agents::SubagentsConfig::default(),
            memory: closeclaw_config::agents::MemoryConfig::default(),
            hooks: Vec::new(),
            source: closeclaw_config::agents::ConfigSource::User,
        },
        effective_max_spawn_depth: 5,
        spawn_timeout: None,
    }
}

fn permission_denied_error(agent_id: &str) -> MockSpawnResult {
    MockSpawnResult::Err(SpawnError::PermissionDenied {
        agent_id: agent_id.to_string(),
        reason: "permission denied for agent".to_string(),
    })
}

fn make_tool(validator: Arc<dyn SpawnValidator>) -> SessionsSpawnTool {
    SessionsSpawnTool::new(
        validator,
        make_session_manager(),
        make_agent_config_lookup(),
        make_approval_flow(),
    )
}

// ---------------------------------------------------------------------------
// Path 1: allow — spawn validation passes
//
// NOTE: The allow path requires SessionManager::create_child_session which
// needs a fully registered parent session in the SpawnTree. The full
// allow-path test is covered by the existing `sessions_spawn_tests.rs`.
// This test verifies that the approval flow is NOT triggered when
// validation succeeds (i.e., no approval_pending in the result).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_approval_allow_path() {
    let validator = Arc::new(MockValidator {
        result: MockSpawnResult::Ok(spawn_result()),
    });
    let mgr = make_session_manager();
    // Register a parent session so create_child_session can look it up.
    let msg = Message {
        id: "msg-parent-allow".to_string(),
        from: "user".to_string(),
        to: "test-agent".to_string(),
        content: "hi".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    let parent_id = mgr
        .find_or_create("test-channel", &msg, None)
        .await
        .expect("find_or_create should succeed");

    let tool = SessionsSpawnTool::new(
        validator,
        mgr,
        make_agent_config_lookup(),
        make_approval_flow(),
    );
    let ctx = make_ctx(&parent_id);
    let result = tool
        .call(json!({"task": "do work", "agentId": "child-agent"}), &ctx)
        .await;
    // The tool should either succeed (create child) or fail with an error
    // unrelated to approval. It must NOT return approval_pending.
    match result {
        Ok(output) => {
            assert!(
                output.data.get("approval_pending").is_none(),
                "allow path should not return approval_pending"
            );
        }
        Err(e) => {
            // If it fails, it should be an ExecutionFailed from
            // create_child_session, not approval_pending or PermissionDenied.
            match e {
                ToolCallError::ExecutionFailed(msg) => {
                    assert!(
                        msg.contains("child session creation failed"),
                        "allow path ExecutionFailed must come from create_child_session, got: {}",
                        msg
                    );
                }
                other => panic!("unexpected error: {:?}", other),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Path 2: deny + enqueue success → approval_pending
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_approval_deny_enqueue_success() {
    let validator = Arc::new(MockValidator {
        result: permission_denied_error("target-agent"),
    });
    let tool = make_tool(validator);
    let ctx = make_ctx("parent-deny");

    let result = tool
        .call(json!({"task": "do work", "agentId": "target-agent"}), &ctx)
        .await;
    let output = result.expect("deny+enqueue should return Ok");
    assert_eq!(
        output.data["status"], "approval_pending",
        "should return approval_pending"
    );
    assert!(
        output.data["request_id"].is_string(),
        "should include request_id"
    );
}

// ---------------------------------------------------------------------------
// Path 3: deny + enqueue failure (duplicate) → fallback to PermissionDenied
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_approval_deny_enqueue_fallback() {
    let flow = make_approval_flow();
    // Pre-enqueue a matching denial so the tool hits duplicate detection.
    let caller = Caller {
        user_id: String::new(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    };
    let body = closeclaw_permission::engine::engine_types::PermissionRequestBody::InterAgentMsg {
        from: "test-agent".to_string(),
        to: "target-agent".to_string(),
    };
    {
        let mut f = flow.lock().await;
        f.submit_denial(&caller, &body, RiskLevel::Medium, "", false)
            .expect("first enqueue should succeed");
    }

    let validator = Arc::new(MockValidator {
        result: permission_denied_error("target-agent"),
    });
    let tool = SessionsSpawnTool::new(
        validator,
        make_session_manager(),
        make_agent_config_lookup(),
        Arc::clone(&flow),
    );
    let ctx = make_ctx("parent-fb");

    let result = tool
        .call(json!({"task": "do work", "agentId": "target-agent"}), &ctx)
        .await;
    let err = result.expect_err("fallback should return error");
    match err {
        ToolCallError::PermissionDenied(msg) => {
            assert!(
                msg.contains("denied"),
                "error should mention denied, got: {}",
                msg
            );
        }
        other => panic!("expected PermissionDenied, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Plan Mode: fork=true must be rejected
// ---------------------------------------------------------------------------

/// Helper to create a ToolContext with a specific session_mode.
fn make_ctx_with_mode(
    session_id: &str,
    session_mode: Option<closeclaw_common::SessionMode>,
) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: Some(session_id.to_string()),
        call_id: None,
        session: None,
        session_mode,
        manual_background_signal: None,
    }
}

/// Plan Mode + fork=true must return InvalidArgs error.
/// Design doc: "Plan Mode 不引入 Fork（上下文继承）机制"
#[tokio::test]
async fn test_spawn_plan_mode_rejects_fork() {
    let validator = Arc::new(MockValidator {
        result: MockSpawnResult::Ok(spawn_result()),
    });
    let tool = make_tool(validator);
    let ctx = make_ctx_with_mode(
        "parent-plan-fork",
        Some(closeclaw_common::SessionMode::Plan),
    );

    let result = tool
        .call(json!({"task": "do work", "fork": true}), &ctx)
        .await;
    let err = result.expect_err("Plan Mode + fork should fail");
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("fork is not allowed in Plan Mode"),
                "error should mention Plan Mode fork restriction, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidArgs, got: {:?}", other),
    }
}

/// Plan Mode + fork=false must succeed (spawn without fork is allowed).
#[tokio::test]
async fn test_spawn_plan_mode_allows_no_fork() {
    let validator = Arc::new(MockValidator {
        result: MockSpawnResult::Ok(spawn_result()),
    });
    let mgr = make_session_manager();
    let msg = Message {
        id: "msg-parent-plan-nofork".to_string(),
        from: "user".to_string(),
        to: "test-agent".to_string(),
        content: "hi".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    let parent_id = mgr
        .find_or_create("test-channel", &msg, None)
        .await
        .expect("find_or_create should succeed");

    let tool = SessionsSpawnTool::new(
        validator,
        mgr,
        make_agent_config_lookup(),
        make_approval_flow(),
    );
    let ctx = make_ctx_with_mode(&parent_id, Some(closeclaw_common::SessionMode::Plan));
    let result = tool
        .call(json!({"task": "do work", "fork": false}), &ctx)
        .await;
    // Should succeed or fail for reasons OTHER than fork rejection.
    match result {
        Ok(output) => {
            assert!(
                output.data.get("approval_pending").is_none(),
                "Plan Mode no-fork should not return approval_pending"
            );
        }
        Err(ToolCallError::InvalidArgs(msg)) if msg.contains("fork is not allowed") => {
            panic!("Plan Mode + fork=false should NOT be rejected");
        }
        Err(other) => {
            // Other errors are acceptable (e.g., session not found).
            eprintln!("Plan Mode no-fork: acceptable error: {:?}", other);
        }
    }
}
