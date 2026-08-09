//! Integration Tests for Permission Engine
//!
//! Tests in this file verify permission engine behavior:
//! - Permission engine + agent registry integration
//! - User/agent dual-key matching
//! - Rule hot-reload
//! - Template resolution

use std::collections::HashMap;
use tempfile::TempDir;

use closeclaw_permission::engine::{
    Action, Caller, CommandArgs, Effect, MatchType, PermissionEngine, PermissionRequest,
    PermissionRequestBody, PermissionResponse, RuleSet,
};
use closeclaw_permission::rules::{RuleBuilder, RuleSetBuilder};

/// Creates a ruleset that grants permissions to any agent.
/// Uses both AgentOnly (glob) and UserAndAgent subjects so that the two-phase
/// merge in `evaluate()` can produce an Allow result.
fn make_test_ruleset() -> RuleSet {
    RuleSetBuilder::new()
        // Agent-phase rules (Subject::AgentOnly)
        .rule(
            RuleBuilder::new()
                .name("allow-file-read-agent")
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
                .name("allow-git-command-agent")
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
                .name("deny-dangerous-git-agent")
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
        // User-phase rules (Subject::UserAndAgent)
        .rule(
            RuleBuilder::new()
                .name("allow-file-read-user")
                .subject_user_and_agent("*", "*", MatchType::Glob, MatchType::Glob)
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
                .name("allow-git-command-user")
                .subject_user_and_agent("*", "*", MatchType::Glob, MatchType::Glob)
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
                .name("deny-dangerous-git-user")
                .subject_user_and_agent("*", "*", MatchType::Glob, MatchType::Glob)
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
        .default_file_read(Effect::Deny)
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

#[test]
fn test_permission_engine_with_registered_agent() {
    let rules = make_test_ruleset();
    let engine = PermissionEngine::new_with_default_data_root(rules);
    let agent_id = "test-agent".to_string();

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
        // Agent-phase: all agents get file read and exec (base permissions)
        .rule(
            RuleBuilder::new()
                .name("dev-read-only-agent")
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
                .name("allow-all-agent")
                .subject_glob("*")
                .allow()
                .action(Action::All)
                .build()
                .unwrap(),
        )
        // User-phase: admin gets full access
        .rule(
            RuleBuilder::new()
                .name("admin-full-access-user")
                .subject_user_and_agent("ou_admin", "*", MatchType::Exact, MatchType::Glob)
                .allow()
                .action(Action::All)
                .build()
                .unwrap(),
        )
        // User-phase: all users get file read
        .rule(
            RuleBuilder::new()
                .name("dev-read-only-user")
                .subject_user_and_agent("*", "*", MatchType::Glob, MatchType::Glob)
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["/home/**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
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
        // Agent-phase rule
        .rule(
            RuleBuilder::new()
                .name("initial-allow-agent")
                .subject_glob("*")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec![tmp_pattern.clone()],
                })
                .build()
                .unwrap(),
        )
        // User-phase rule
        .rule(
            RuleBuilder::new()
                .name("initial-allow-user")
                .subject_user_and_agent("*", "*", MatchType::Glob, MatchType::Glob)
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec![tmp_pattern],
                })
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
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
        .default_file_read(Effect::Deny)
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

#[tokio::test]
async fn test_permission_engine_template_resolution() {
    // Create template
    let mut templates = HashMap::new();
    templates.insert(
        "developer".to_string(),
        closeclaw_permission::templates::Template {
            name: "developer".to_string(),
            description: "Standard developer permissions".to_string(),
            subject: closeclaw_permission::templates::TemplateSubject::Any,
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
        // Agent-phase rule
        .rule(
            RuleBuilder::new()
                .name("developer-via-template-agent")
                .subject_glob("*")
                .allow()
                .template("developer")
                .build()
                .unwrap(),
        )
        // User-phase rule
        .rule(
            RuleBuilder::new()
                .name("developer-via-template-user")
                .subject_user_and_agent("*", "*", MatchType::Glob, MatchType::Glob)
                .allow()
                .template("developer")
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
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
