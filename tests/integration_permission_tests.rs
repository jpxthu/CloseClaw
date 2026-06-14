//! Integration Tests for Permission Engine
//!
//! Tests in this file verify permission engine behavior:
//! - Permission engine + agent registry integration
//! - User/agent dual-key matching
//! - Rule hot-reload
//! - Template resolution

use std::collections::HashMap;
use tempfile::TempDir;

use closeclaw::agent::registry::{create_registry, SharedAgentRegistry};
use closeclaw::permission::engine::{
    Action, Caller, CommandArgs, Effect, MatchType, PermissionEngine, PermissionRequest,
    PermissionRequestBody, PermissionResponse, RuleSet,
};
use closeclaw::permission::rules::{RuleBuilder, RuleSetBuilder};

/// Creates a ruleset that grants permissions to any agent (using wildcard patterns).
fn make_test_ruleset() -> RuleSet {
    RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("allow-file-read")
                .subject_glob("*")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["/home/**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("allow-git-command")
                .subject_glob("*")
                .allow()
                .action(Action::Command {
                    command: "git".to_string(),
                    args: CommandArgs::Allowed {
                        allowed: vec![
                            "status".to_string(),
                            "log".to_string(),
                            "diff".to_string(),
                            "add".to_string(),
                            "commit".to_string(),
                        ],
                    },
                })
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("deny-dangerous-git")
                .subject_glob("*")
                .deny()
                .action(Action::Command {
                    command: "git".to_string(),
                    args: CommandArgs::Blocked {
                        blocked: vec!["reset --hard".to_string(), "push --force".to_string()],
                    },
                })
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("allow-tool-call-file-ops")
                .subject_glob("*")
                .allow()
                .action(Action::ToolCall {
                    skill: "file_ops".to_string(),
                    methods: vec!["read".to_string(), "exists".to_string()],
                })
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("allow-tool-call-git-ops")
                .subject_glob("*")
                .allow()
                .action(Action::ToolCall {
                    skill: "git_ops".to_string(),
                    methods: vec!["status".to_string(), "log".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Integration Test: Permission Engine + Agent Registry
// End-to-end: Permission checks for registered agents
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_permission_engine_with_registered_agent() {
    let registry: SharedAgentRegistry = create_registry(30);
    let rules = make_test_ruleset();
    let engine = PermissionEngine::new_with_default_data_root(rules);

    // Register an agent (ID will be UUID)
    let agent = registry
        .register("test-agent".to_string(), None)
        .await
        .unwrap();
    let agent_id = agent.id.clone();

    // Verify file read is allowed (wildcard subject matches any agent)
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: agent_id.clone(),
            path: "/home/admin/code/main.rs".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Verify exec is denied (no exec rule)
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: agent_id.clone(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Denied { .. }));

    // Verify git status is allowed (has git command rule)
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: agent_id.clone(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Verify dangerous git args are denied
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: agent_id.clone(),
            cmd: "git".to_string(),
            args: vec!["reset".to_string(), "--hard".to_string()],
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_user_and_agent_dual_key_matching() {
    // Build ruleset with dual-key subject
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("admin-full-access")
                .subject_user_and_agent("ou_admin", "*", MatchType::Exact, MatchType::Glob)
                .allow()
                .action(Action::All)
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("dev-read-only")
                .subject_glob("*")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["/home/**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // Admin user with any agent should get full access (including exec)
    let response = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "ou_admin".to_string(),
                agent: "any-agent".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::CommandExec {
                agent: "any-agent".to_string(),
                cmd: "rm".to_string(),
                args: vec!["-rf".to_string(), "/".to_string()],
            },
        },
        None,
    );
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Non-admin user should be denied exec
    let response = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "ou_regular_user".to_string(),
                agent: "any-agent".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::CommandExec {
                agent: "any-agent".to_string(),
                cmd: "rm".to_string(),
                args: vec!["-rf".to_string(), "/".to_string()],
            },
        },
        None,
    );
    assert!(matches!(response, PermissionResponse::Denied { .. }));

    // But regular user should still have file read permission via agent-only rule
    let response = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "ou_regular_user".to_string(),
                agent: "any-agent".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "any-agent".to_string(),
                path: "/home/admin/file.txt".to_string(),
                op: "read".to_string(),
            },
        },
        None,
    );
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_engine_reload_updates_rules() {
    let tmpdir = TempDir::new().unwrap();
    let tmp_pattern = format!("{}/**", tmpdir.path().to_string_lossy());
    let initial_rules = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("initial-allow")
                .subject_glob("*")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec![tmp_pattern],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let mut engine = PermissionEngine::new_with_default_data_root(initial_rules);

    // Initial: should allow
    let test_file = tmpdir.path().join("file.txt");
    let test_file_str = test_file.to_str().unwrap().to_string();
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "any-agent".to_string(),
            path: test_file_str.clone(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Reload with new rules (no rules, default deny)
    let new_rules = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    engine.reload_rules(new_rules);

    // After reload: should deny
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "any-agent".to_string(),
            path: test_file_str,
            op: "read".to_string(),
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

// ---------------------------------------------------------------------------
// Integration Test: Permission Engine Template Resolution
// Ensures templates are properly resolved during rule expansion
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Regression: Bare request with AgentOnly rules should return Allowed
// Verifies Step 1.1 fix: (Some(Allowed), None) no longer falls to default_deny
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_bare_request_agent_only_rules_return_allowed() {
    // Build a ruleset with ONLY AgentOnly rules (no UserAndAgent rules)
    let rules = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("agent-read-file")
                .subject_glob("*")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["/home/**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("agent-exec-git")
                .subject_glob("*")
                .allow()
                .action(Action::Command {
                    command: "git".to_string(),
                    args: CommandArgs::Allowed {
                        allowed: vec!["status".to_string(), "log".to_string()],
                    },
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(rules);

    // Bare file read — agent allows, no user rules → should be Allowed
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/home/admin/file.txt".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(
        matches!(response, PermissionResponse::Allowed { .. }),
        "Bare request with only AgentOnly rules should return Allowed, got: {:?}",
        response
    );

    // Bare git status — agent allows, no user rules → should be Allowed
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        }),
        None,
    );
    assert!(
        matches!(response, PermissionResponse::Allowed { .. }),
        "Bare request with only AgentOnly rules should return Allowed, got: {:?}",
        response
    );

    // Bare exec rm — no matching agent rule → should be Denied (default_deny)
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        }),
        None,
    );
    assert!(
        matches!(response, PermissionResponse::Denied { .. }),
        "Bare request with no matching rule should return Denied, got: {:?}",
        response
    );
}

#[tokio::test]
async fn test_permission_engine_template_resolution() {
    // Create template
    let mut templates = HashMap::new();
    templates.insert(
        "developer".to_string(),
        closeclaw::permission::templates::Template {
            name: "developer".to_string(),
            description: "Standard developer permissions".to_string(),
            subject: closeclaw::permission::templates::TemplateSubject::Any,
            effect: Effect::Allow,
            actions: vec![
                Action::File {
                    operation: "read".to_string(),
                    paths: vec!["/home/**".to_string()],
                },
                Action::Command {
                    command: "git".to_string(),
                    args: CommandArgs::Allowed {
                        allowed: vec![
                            "status".to_string(),
                            "log".to_string(),
                            "diff".to_string(),
                            "add".to_string(),
                            "commit".to_string(),
                        ],
                    },
                },
                Action::ToolCall {
                    skill: "file_ops".to_string(),
                    methods: vec!["read".to_string(), "exists".to_string()],
                },
            ],
            extends: vec![],
        },
    );

    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("developer-via-template")
                .subject_glob("*")
                .allow()
                .template("developer")
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();

    let mut engine = PermissionEngine::new_with_default_data_root(ruleset);
    engine.load_templates(templates);

    // Test file read via template-resolved rule
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "any-agent".to_string(),
            path: "/home/admin/code/main.rs".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Test git status via template-resolved rule
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "any-agent".to_string(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Test unknown command should still be denied (no default allow)
    let response = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "any-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        }),
        None,
    );
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}
