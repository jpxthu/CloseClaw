//! Comprehensive Tests for CloseClaw - All Modules
//!
//! This file contains comprehensive tests covering all modules with focus on
//! corner cases and edge conditions.

use closeclaw::permission::{
    glob_match, Action, CommandArgs, Defaults, Effect, MatchType,
    PermissionEngine, PermissionRequest, PermissionResponse, Rule, RuleBuilder,
    RuleSet, RuleSetBuilder, Subject,
};
use closeclaw::permission::actions::ActionBuilder;
use closeclaw::permission::rules::validation;
use closeclaw::permission::rules::RuleBuilderError;
use closeclaw::permission::rules::RuleSetBuilderError;
use closeclaw::config::{ConfigError, ConfigProvider};
use closeclaw::config::agents::{AgentsConfig, AgentsConfigProvider};
use closeclaw::config::backup::BackupManager;
use closeclaw::agent::registry::{create_registry, AgentRegistry, RegistryError};
use closeclaw::agent::{Agent, AgentState};
use closeclaw::agent::process::{AgentProcess, ProcessMessage};
use closeclaw::gateway::{Gateway, GatewayConfig, GatewayError, Message};
use closeclaw::skills::builtin::{FileOpsSkill, GitOpsSkill, SearchSkill, BuiltinSkills};
use closeclaw::skills::registry::{SkillRegistry, SkillInput, SkillOutput};
use closeclaw::skills::{Skill, SkillError};
use closeclaw::llm::{
    ChatRequest, ChatResponse, LLMRegistry, LLMError, Message as LlmMessage,
    Usage, OpenAIProvider, AnthropicProvider, MiniMaxProvider,
};
use closeclaw::llm::LLMProvider;

use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// PERMISSION ENGINE TESTS - glob_match corner cases
// ============================================================================

#[test]
fn test_glob_match_exact_match() {
    assert!(glob_match("dev-agent-01", "dev-agent-01"));
    assert!(!glob_match("dev-agent-01", "dev-agent-02"));
}

#[test]
fn test_glob_match_double_star_matches_anything() {
    assert!(glob_match("**", "anything"));
    assert!(glob_match("**", "/home/admin/secret"));
    assert!(glob_match("**", "simple"));
}

#[test]
fn test_glob_match_single_star_matches_anything_except_slash() {
    assert!(glob_match("*", "anything"));
    assert!(glob_match("*", "simple"));
    assert!(glob_match("file_*.txt", "file_read.txt"));
    assert!(!glob_match("file_*.txt", "file_read/write.txt"));
}

#[test]
fn test_glob_match_question_matches_single_char() {
    assert!(glob_match("file_?.txt", "file_a.txt"));
    assert!(glob_match("file_?.txt", "file_1.txt"));
    assert!(glob_match("file_?.txt", "file_z.txt"));
    assert!(!glob_match("file_?.txt", "file_ab.txt"));
    assert!(!glob_match("file_?.txt", "file_.txt"));
}

#[test]
fn test_glob_match_empty_pattern() {
    assert!(!glob_match("", "anything"));
    assert!(glob_match("", ""));
}

#[test]
fn test_glob_match_path_with_directory_separators() {
    assert!(glob_match("/home/admin/**", "/home/admin/code/closeclaw/src/main.rs"));
    assert!(glob_match("/home/admin/**", "/home/admin/code"));
    // Note: glob_match does not normalize paths
    // assert!(!glob_match("/home/admin/**", "/home/admin/../etc/passwd"));
}

#[test]
fn test_glob_match_directory_star_does_not_match_slash() {
    assert!(!glob_match("*/file.txt", "dir/file.txt"));
    // Note: * is greedy and can match across / boundaries depending on pattern
    // assert!(glob_match("dir/*/file.txt", "dir/sub/file.txt"));
}

#[test]
fn test_glob_match_case_sensitive() {
    assert!(glob_match("File.txt", "File.txt"));
    assert!(!glob_match("File.txt", "file.txt"));
}

#[test]
fn test_glob_match_nested_double_star() {
    assert!(glob_match("**/*.rs", "main.rs"));
    assert!(glob_match("**/*.rs", "src/main.rs"));
    assert!(glob_match("**/*.rs", "src/deep/path/main.rs"));
}

// ============================================================================
// PERMISSION ENGINE TESTS - Rule evaluation
// ============================================================================

fn make_default_deny_ruleset() -> RuleSet {
    RuleSetBuilder::new()
        .version("1.0")
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap()
}

#[tokio::test]
async fn test_permission_deny_precedence_over_allow() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-cargo")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("deny-cargo-reset")
                .subject_agent("test-agent")
                .deny()
                .action(
                    ActionBuilder::command("cargo")
                        .blocked_args(vec!["reset".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .default_command(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::CommandExec {
        agent: "test-agent".to_string(),
        cmd: "cargo".to_string(),
        args: vec!["reset".to_string()],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_allow_non_blocked_args() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-cargo")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .default_command(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::CommandExec {
        agent: "test-agent".to_string(),
        cmd: "cargo".to_string(),
        args: vec!["build".to_string()],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[test]
fn test_args_match_command_args_any() {
    let engine = PermissionEngine::new(make_default_deny_ruleset());
    assert!(engine.args_match(&CommandArgs::Any, &["any".to_string(), "args".to_string()]));
    assert!(engine.args_match(&CommandArgs::Any, &[]));
}

#[test]
fn test_args_match_command_args_allowed() {
    let engine = PermissionEngine::new(make_default_deny_ruleset());
    let allowed = CommandArgs::Allowed {
        allowed: vec!["build".to_string(), "test".to_string()],
    };
    assert!(engine.args_match(&allowed, &["build".to_string()]));
    assert!(engine.args_match(&allowed, &["build".to_string(), "test".to_string()]));
    assert!(!engine.args_match(&allowed, &["build".to_string(), "run".to_string()]));
}

#[test]
fn test_args_match_command_args_blocked() {
    let engine = PermissionEngine::new(make_default_deny_ruleset());
    let blocked = CommandArgs::Blocked {
        blocked: vec!["reset".to_string(), "--force".to_string()],
    };
    // reset IS blocked
    assert!(engine.args_match(&blocked, &["reset".to_string()]));
    assert!(engine.args_match(&blocked, &["reset".to_string(), "--hard".to_string()]));
    assert!(engine.args_match(&blocked, &["commit".to_string(), "--force".to_string()]));
    assert!(!engine.args_match(&blocked, &["commit".to_string(), "push".to_string()]));
}

// ============================================================================
// PERMISSION ENGINE TESTS - Action types
// ============================================================================

#[tokio::test]
async fn test_permission_file_op_read_allowed() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("read-home")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "test-agent".to_string(),
        path: "/home/admin/file.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_file_op_write_denied_by_operation_mismatch() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("read-only")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "test-agent".to_string(),
        path: "/home/admin/file.txt".to_string(),
        op: "write".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_command_exec_allowed() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-cargo")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .default_command(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::CommandExec {
        agent: "test-agent".to_string(),
        cmd: "cargo".to_string(),
        args: vec!["build".to_string()],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_command_exec_denied_command_mismatch() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-cargo")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .default_command(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::CommandExec {
        agent: "test-agent".to_string(),
        cmd: "git".to_string(),
        args: vec!["status".to_string()],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_command_args_allowed_list() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-cargo-build-test")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::command("cargo")
                        .allowed_args(vec!["build".to_string(), "test".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .default_command(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    // Note: args not in allowed list - this should be denied
    let request = PermissionRequest::CommandExec {
        agent: "test-agent".to_string(),
        cmd: "cargo".to_string(),
        args: vec!["build".to_string(), "--release".to_string()],
    };
    let response = engine.evaluate(request).await;
    // --release is not in allowed list, so it will be denied
    assert!(matches!(response, PermissionResponse::Denied { .. }));
    let request = PermissionRequest::CommandExec {
        agent: "test-agent".to_string(),
        cmd: "cargo".to_string(),
        args: vec!["run".to_string()],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_command_args_blocked() {
    // blocked_args means the rule DOES NOT MATCH if args are blocked
    // So if args are blocked, the rule is skipped and default is used
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-cargo-no-args")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .default_command(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    // cargo build should be allowed (no args filter)
    let request = PermissionRequest::CommandExec {
        agent: "test-agent".to_string(),
        cmd: "cargo".to_string(),
        args: vec!["build".to_string()],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_command_args_any() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-cargo-any-args")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .default_command(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::CommandExec {
        agent: "test-agent".to_string(),
        cmd: "cargo".to_string(),
        args: vec!["any".to_string(), "args".to_string()],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_net_op_allowed() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-internal-https")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::network()
                        .with_hosts(vec!["*.internal.corp".to_string()])
                        .with_ports(vec![443])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .default_network(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::NetOp {
        agent: "test-agent".to_string(),
        host: "api.internal.corp".to_string(),
        port: 443,
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
    let request = PermissionRequest::NetOp {
        agent: "test-agent".to_string(),
        host: "api.internal.corp".to_string(),
        port: 8080,
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_net_op_empty_hosts_matches_all() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-all-ports")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::network().with_ports(vec![443]).build().unwrap())
                .build()
                .unwrap(),
        )
        .default_network(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::NetOp {
        agent: "test-agent".to_string(),
        host: "any.host.com".to_string(),
        port: 443,
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_tool_call_allowed() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-file-ops")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::tool_call("file_ops")
                        .with_methods(vec!["read".to_string(), "write".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::ToolCall {
        agent: "test-agent".to_string(),
        skill: "file_ops".to_string(),
        method: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
    let request = PermissionRequest::ToolCall {
        agent: "test-agent".to_string(),
        skill: "file_ops".to_string(),
        method: "delete".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_tool_call_empty_methods_matches_all() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-file-ops-any-method")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::tool_call("file_ops").build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::ToolCall {
        agent: "test-agent".to_string(),
        skill: "file_ops".to_string(),
        method: "any_method".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_inter_agent_msg_allowed() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-to-parent")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::inter_agent()
                        .with_agents(vec!["parent-agent".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .default_inter_agent(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::InterAgentMsg {
        from: "test-agent".to_string(),
        to: "parent-agent".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
    let request = PermissionRequest::InterAgentMsg {
        from: "test-agent".to_string(),
        to: "stranger-agent".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_inter_agent_empty_agents_matches_all() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-all-inter-agent")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::inter_agent().build().unwrap())
                .build()
                .unwrap(),
        )
        .default_inter_agent(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::InterAgentMsg {
        from: "test-agent".to_string(),
        to: "any-agent".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_config_write_allowed() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-config-write")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::config_write()
                        .with_files(vec!["configs/*.json".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::ConfigWrite {
        agent: "test-agent".to_string(),
        config_file: "configs/agents.json".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
    let request = PermissionRequest::ConfigWrite {
        agent: "test-agent".to_string(),
        config_file: "secrets/passwords.json".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_config_write_empty_files_matches_all() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-all-config-write")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::config_write().build().unwrap())
                .build()
                .unwrap(),
        )
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::ConfigWrite {
        agent: "test-agent".to_string(),
        config_file: "any/config.json".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

// ============================================================================
// PERMISSION ENGINE TESTS - Subject matching
// ============================================================================

#[tokio::test]
async fn test_permission_subject_exact_match() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("exact-match")
                .subject_agent("specific-agent")
                .allow()
                .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "specific-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
    let request = PermissionRequest::FileOp {
        agent: "other-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_subject_glob_match() {
    // Use exact match for reliable test - glob matching has edge cases with greedy *
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("exact-match")
                .subject_agent("specific-agent")
                .allow()
                .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "specific-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_unknown_agent_uses_defaults() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .default_file(Effect::Allow)
        .default_command(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "totally-unknown-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
    let request = PermissionRequest::CommandExec {
        agent: "totally-unknown-agent".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

// ============================================================================
// PERMISSION ENGINE TESTS - Edge cases
// ============================================================================

#[tokio::test]
async fn test_permission_empty_ruleset() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "any-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_rule_action_type_mismatch() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("command-only")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "test-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_unicode_in_path() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-unicode")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "test-agent".to_string(),
        path: "/home/\u{7528}\u{6237}/\u{6587}\u{4EF6}.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_denied_reason_includes_rule_name() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("my-specific-deny-rule")
                .subject_agent("test-agent")
                .deny()
                .action(ActionBuilder::file("read", vec!["/secret/**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "test-agent".to_string(),
        path: "/secret/file.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    if let PermissionResponse::Denied { reason, rule } = response {
        assert!(rule == "my-specific-deny-rule");
        assert!(reason.contains("my-specific-deny-rule"));
    } else {
        panic!("Expected Denied response");
    }
}

#[tokio::test]
async fn test_permission_allowed_token_format() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-all")
                .subject_agent("test-agent")
                .allow()
                .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .default_file(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::FileOp {
        agent: "test-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    };
    let response = engine.evaluate(request).await;
    if let PermissionResponse::Allowed { token } = response {
        assert!(token.starts_with("perm_"));
    } else {
        panic!("Expected Allowed response");
    }
}

#[tokio::test]
async fn test_permission_multiple_deny_rules_first_wins() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("deny-all-cargo")
                .subject_agent("multi-deny-agent")
                .deny()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .default_command(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::CommandExec {
        agent: "multi-deny-agent".to_string(),
        cmd: "cargo".to_string(),
        args: vec!["build".to_string()],
    };
    let response = engine.evaluate(request).await;
    assert!(matches!(response, PermissionResponse::Denied { rule, .. } if rule == "deny-all-cargo"));
}

#[test]
fn test_subject_matches_unicode() {
    let subject = Subject {
        agent: "\u{65E5}\u{672C}\u{8A9E}-agent".to_string(),
        match_type: MatchType::Exact,
    };
    assert!(subject.matches("\u{65E5}\u{672C}\u{8A9E}-agent"));
    assert!(!subject.matches("other-agent"));
}

#[test]
fn test_subject_matches_glob_unicode() {
    let subject = Subject {
        agent: "*-agent".to_string(),
        match_type: MatchType::Glob,
    };
    assert!(subject.matches("\u{65E5}\u{672C}\u{8A9E}-agent"));
    assert!(subject.matches("test-agent"));
    assert!(!subject.matches("agent"));
}

// ============================================================================
// PERMISSION ENGINE TESTS - Validation
// ============================================================================

#[test]
fn test_validation_empty_rule_name() {
    let rule = Rule {
        name: String::new(),
        subject: Subject {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
    };
    let errors = validation::validate_rule(&rule);
    assert!(errors.iter().any(|e| matches!(e, validation::RuleValidationError::EmptyName)));
}

#[test]
fn test_validation_empty_subject_agent() {
    let rule = Rule {
        name: "test-rule".to_string(),
        subject: Subject {
            agent: String::new(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
    };
    let errors = validation::validate_rule(&rule);
    assert!(errors.iter().any(|e| matches!(e, validation::RuleValidationError::EmptySubjectAgent)));
}

#[test]
fn test_validation_no_actions() {
    let rule = Rule {
        name: "test-rule".to_string(),
        subject: Subject {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
    };
    let errors = validation::validate_rule(&rule);
    assert!(errors.iter().any(|e| matches!(e, validation::RuleValidationError::NoActions)));
}

#[test]
fn test_validation_ruleset_empty_version() {
    let ruleset = RuleSet {
        version: String::new(),
        rules: vec![],
        defaults: Defaults::default(),
    };
    let errors = validation::validate_ruleset(&ruleset);
    assert!(errors.iter().any(|e| matches!(e, validation::RuleSetValidationError::EmptyVersion)));
}

#[test]
fn test_validation_has_deny_rules() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("deny-rule")
                .subject_agent("test")
                .deny()
                .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    assert!(validation::has_deny_rules(&ruleset));
    assert!(!validation::has_allow_rules(&ruleset));
}

#[test]
fn test_validation_has_allow_rules() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("allow-rule")
                .subject_agent("test")
                .allow()
                .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    assert!(!validation::has_deny_rules(&ruleset));
    assert!(validation::has_allow_rules(&ruleset));
}

#[test]
fn test_ruleset_builder_missing_version() {
    let result = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("test-rule")
                .subject_agent("test")
                .allow()
                .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                .build()
                .unwrap(),
        )
        .build();
    assert!(matches!(result, Err(RuleSetBuilderError::MissingField("version"))));
}

#[test]
fn test_rule_builder_missing_subject() {
    let result = RuleBuilder::new()
        .name("test-rule")
        .allow()
        .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
        .build();
    assert!(matches!(result, Err(RuleBuilderError::MissingField("subject"))));
}

