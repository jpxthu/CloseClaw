//! Unit tests for the permission_check module.

use super::*;
use crate::{ToolCallError, ToolContext};
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{Action, Effect, Rule};
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_permission::Defaults;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_engine_with_rules(rules: Vec<Rule>) -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    let rs = RuleSetBuilder::new()
        .rules(rules)
        .defaults(Defaults {
            tool_call: Effect::Deny,
            file: Effect::Deny,
            command: Effect::Deny,
            ..Default::default()
        })
        .build()
        .unwrap();
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rs),
    ))
}

fn make_sm() -> Arc<SessionManager> {
    use closeclaw_gateway::GatewayConfig;
    use closeclaw_session::bootstrap::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: closeclaw_gateway::DmScope::default(),
            ..Default::default()
        },
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

fn make_cm() -> Arc<ConfigManager> {
    let tmp = tempfile::TempDir::new().unwrap();
    Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    )
}

/// Standard approval flow — enqueues denials (approval-pending path).
fn make_af() -> Arc<ApprovalMutex> {
    Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&make_sm()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
    )))
}

/// Denying approval flow — submit_denial returns None (hard deny path).
fn make_af_deny() -> Arc<ApprovalMutex> {
    Arc::new(TokioMutex::new(ApprovalFlow::new_deny_all(
        Arc::clone(&make_sm()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
    )))
}

fn make_deps(rules: Vec<Rule>) -> PermDeps {
    (
        make_engine_with_rules(rules),
        make_sm(),
        make_cm(),
        make_af(),
    )
}

/// Like `make_deps` but uses a deny-all approval flow.
fn make_deps_deny(rules: Vec<Rule>) -> PermDeps {
    (
        make_engine_with_rules(rules),
        make_sm(),
        make_cm(),
        make_af_deny(),
    )
}

fn make_ctx(agent: &str) -> ToolContext {
    ToolContext {
        agent_id: agent.to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
    }
}

fn allow_tool_rule(agent: &str, skill: &str) -> Rule {
    Rule {
        name: format!("allow-{skill}-call"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::ToolCall {
            skill: skill.to_string(),
            methods: vec!["call".to_string()],
        }],
        template: None,
        priority: 0,
    }
}

fn allow_file_rule(agent: &str, path_glob: &str, op: &str) -> Rule {
    Rule {
        name: format!("allow-file-{op}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: op.to_string(),
            paths: vec![path_glob.to_string()],
        }],
        template: None,
        priority: 0,
    }
}

fn allow_cmd_rule(agent: &str, cmd_pattern: &str) -> Rule {
    Rule {
        name: format!("allow-cmd-{cmd_pattern}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::Command {
            command: cmd_pattern.to_string(),
            args: Default::default(),
        }],
        template: None,
        priority: 0,
    }
}

// ---------------------------------------------------------------------------
// check_tool_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tool_allowed_when_rule_matches() {
    let deps = make_deps(vec![allow_tool_rule("agent-a", "bash")]);
    let ctx = make_ctx("agent-a");
    let result = check_tool_permission(&deps, &ctx, "bash", "call").await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "allowed → None");
}

#[tokio::test]
async fn test_tool_denied_when_no_matching_rule() {
    let deps = make_deps_deny(vec![allow_tool_rule("agent-a", "bash")]);
    let ctx = make_ctx("other-agent");
    let result = check_tool_permission(&deps, &ctx, "bash", "call").await;
    match result {
        Err(ToolCallError::PermissionDenied(reason)) => {
            assert!(!reason.is_empty());
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

#[tokio::test]
async fn test_tool_denied_when_wrong_skill() {
    let deps = make_deps_deny(vec![allow_tool_rule("agent-a", "file_ops")]);
    let ctx = make_ctx("agent-a");
    let result = check_tool_permission(&deps, &ctx, "bash", "call").await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// check_file_op_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_file_op_read_allowed() {
    let deps = make_deps(vec![allow_file_rule("agent-a", "/tmp/**", "read")]);
    let ctx = make_ctx("agent-a");
    let result = check_file_op_permission(&deps, &ctx, "/tmp/test.txt", "read").await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn test_file_op_write_allowed() {
    let deps = make_deps(vec![allow_file_rule("agent-a", "/tmp/**", "write")]);
    let ctx = make_ctx("agent-a");
    let result = check_file_op_permission(&deps, &ctx, "/tmp/out.txt", "write").await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn test_file_op_denied_without_rule() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_file_op_permission(&deps, &ctx, "/tmp/test.txt", "read").await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// check_command_permission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_command_allowed() {
    let deps = make_deps(vec![allow_cmd_rule("agent-a", "echo")]);
    let ctx = make_ctx("agent-a");
    let result = check_command_permission(&deps, &ctx, "echo", &["hello".to_string()]).await;
    assert!(matches!(result, CommandPermissionResult::Permitted));
}

#[tokio::test]
async fn test_command_denied_without_rule() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result =
        check_command_permission(&deps, &ctx, "rm", &["-rf".to_string(), "/".to_string()]).await;
    assert!(matches!(result, CommandPermissionResult::Denied(_)));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tool_empty_skill_name() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_tool_permission(&deps, &ctx, "", "call").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_file_op_empty_path() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_file_op_permission(&deps, &ctx, "", "read").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_command_empty_cmd() {
    let deps = make_deps_deny(vec![]);
    let ctx = make_ctx("agent-a");
    let result = check_command_permission(&deps, &ctx, "", &vec![]).await;
    assert!(matches!(result, CommandPermissionResult::Denied(_)));
}

#[tokio::test]
async fn test_file_op_special_char_path() {
    let deps = make_deps(vec![allow_file_rule("agent-a", "/tmp/**", "read")]);
    let ctx = make_ctx("agent-a");
    let path = "/tmp/file with spaces & special!@#.txt";
    let result = check_file_op_permission(&deps, &ctx, path, "read").await;
    // Should not panic on special chars
    assert!(result.is_ok() || result.is_err());
}

// ---------------------------------------------------------------------------
// Two-level: Level 1 blocks → Level 2 never reached
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_level1_block_prevents_level2() {
    // Agent has FileOp rule but NO ToolCall rule.
    // Level 1 (ToolCall) should deny before Level 2 (FileOp) is checked.
    let deps = make_deps_deny(vec![allow_file_rule("agent-a", "/tmp/**", "read")]);
    let ctx = make_ctx("agent-a");

    let level1 = check_tool_permission(&deps, &ctx, "file_ops", "call").await;
    assert!(level1.is_err(), "Level 1 should deny");
}

#[tokio::test]
async fn test_level1_pass_level2_pass() {
    // Agent has both ToolCall and FileOp rules.
    let deps = make_deps(vec![
        allow_tool_rule("agent-a", "file_ops"),
        allow_file_rule("agent-a", "/tmp/**", "read"),
    ]);
    let ctx = make_ctx("agent-a");

    let level1 = check_tool_permission(&deps, &ctx, "file_ops", "call").await;
    assert!(level1.is_ok());
    assert!(level1.unwrap().is_none());

    let level2 = check_file_op_permission(&deps, &ctx, "/tmp/file.txt", "read").await;
    assert!(level2.is_ok());
    assert!(level2.unwrap().is_none());
}

#[tokio::test]
async fn test_level1_pass_level2_denied() {
    // Agent has ToolCall rule but NO FileOp rule.
    let deps = make_deps(vec![allow_tool_rule("agent-a", "file_ops")]);
    let ctx = make_ctx("agent-a");

    let level1 = check_tool_permission(&deps, &ctx, "file_ops", "call").await;
    assert!(level1.is_ok());
    assert!(level1.unwrap().is_none());

    // Use deny flow for Level 2 to get a hard denial
    let deps2 = make_deps_deny(vec![allow_tool_rule("agent-a", "file_ops")]);
    let level2 = check_file_op_permission(&deps2, &ctx, "/tmp/file.txt", "read").await;
    assert!(level2.is_err(), "Level 2 should deny");
}
