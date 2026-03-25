//! Integration Tests for CloseClaw
//!
//! Tests in this file verify cross-module interactions:
//! - Permission engine + agent registry
//! - Config loading + agent registry
//! - Skill loading + execution chain

use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test imports — from the library crate
// ---------------------------------------------------------------------------
use closeclaw::agent::registry::{create_registry, SharedAgentRegistry};
use closeclaw::agent::AgentState;
use closeclaw::config::agents::AgentsConfigProvider;
use closeclaw::config::ConfigProvider;
use closeclaw::permission::engine::{
    Action, Caller, CommandArgs, Effect, MatchType, PermissionEngine, PermissionRequest,
    PermissionRequestBody, PermissionResponse, RuleSet,
};
use closeclaw::permission::rules::{RuleBuilder, RuleSetBuilder};
use closeclaw::skills::Skill;

// ---------------------------------------------------------------------------
// Helper builders
// ---------------------------------------------------------------------------

/// Creates a ruleset that grants permissions to any agent (using wildcard patterns).
/// This allows testing permission engine + agent registry integration without
/// having to worry about UUID vs name mismatches.
fn make_test_ruleset() -> RuleSet {
    RuleSetBuilder::new()
        .version("1.0.0")
        .rule(
            RuleBuilder::new()
                .name("allow-file-read")
                .subject_glob("*")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["/home/**".to_string(), "/tmp/**".to_string()],
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
// Integration Test 1: Agent Registry Lifecycle
// Tests agent creation, state transitions, parent-child hierarchy, heartbeat
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_registry_lifecycle() {
    // Build registry
    let registry: SharedAgentRegistry = create_registry(30);

    // Register agent
    let agent = registry
        .register("test-agent".to_string(), None)
        .await
        .unwrap();
    let agent_id = agent.id.clone();

    // Verify agent is idle
    let retrieved = registry.get(&agent_id).await.unwrap();
    assert_eq!(retrieved.state, AgentState::Idle);
    assert_eq!(retrieved.name, "test-agent");

    // Transition to Running
    registry
        .update_state(&agent_id, AgentState::Running)
        .await
        .unwrap();

    // Verify state changed
    let retrieved = registry.get(&agent_id).await.unwrap();
    assert_eq!(retrieved.state, AgentState::Running);

    // Transition to Waiting
    registry
        .update_state(&agent_id, AgentState::Waiting)
        .await
        .unwrap();

    // Transition to Suspended
    registry
        .update_state(&agent_id, AgentState::Suspended)
        .await
        .unwrap();

    // Transition to Stopped
    registry
        .update_state(&agent_id, AgentState::Stopped)
        .await
        .unwrap();

    // Verify terminal state - can't transition back to Running
    let result = registry.update_state(&agent_id, AgentState::Running).await;
    assert!(result.is_err());

    // Cleanup
    registry.remove(&agent_id).await.unwrap();

    // Verify agent gone
    assert!(registry.get(&agent_id).await.is_err());
}

#[tokio::test]
async fn test_agent_parent_child_hierarchy() {
    let registry: SharedAgentRegistry = create_registry(30);

    // Register parent
    let parent = registry
        .register("parent-agent".to_string(), None)
        .await
        .unwrap();
    let parent_id = parent.id.clone();

    // Register child
    let child = registry
        .register("child-agent".to_string(), Some(parent_id.clone()))
        .await
        .unwrap();
    let child_id = child.id.clone();

    // Register grandchild
    let grandchild = registry
        .register("grandchild-agent".to_string(), Some(child_id.clone()))
        .await
        .unwrap();
    let grandchild_id = grandchild.id.clone();

    // Verify parent-child relationship
    let children = registry.get_children(&parent_id).await;
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id, child_id);

    let parent_retrieved = registry.get_parent(&child_id).await;
    assert!(parent_retrieved.is_some());
    assert_eq!(parent_retrieved.unwrap().id, parent_id);

    // Verify ancestors chain for grandchild
    let ancestors = registry.get_ancestors(&grandchild_id).await;
    assert_eq!(ancestors.len(), 2);
    assert_eq!(ancestors[0].id, child_id);
    assert_eq!(ancestors[1].id, parent_id);

    // Verify ancestor checks
    assert!(registry.is_ancestor_of(&parent_id, &grandchild_id).await);
    assert!(registry.is_ancestor_of(&child_id, &grandchild_id).await);
    assert!(!registry.is_ancestor_of(&grandchild_id, &parent_id).await);
    assert!(!registry.is_ancestor_of(&parent_id, &parent_id).await);

    // List all agents
    let all = registry.list().await;
    assert_eq!(all.len(), 3);

    // List by state
    let running = registry.list_by_state(AgentState::Running).await;
    assert!(running.is_empty());

    let idle = registry.list_by_state(AgentState::Idle).await;
    assert_eq!(idle.len(), 3);
}

#[tokio::test]
async fn test_heartbeat_timeout_marks_agent_dead() {
    // Create registry with very short timeout (1 second)
    let registry: SharedAgentRegistry = create_registry(1);

    let agent = registry
        .register("test-agent".to_string(), None)
        .await
        .unwrap();
    let agent_id = agent.id.clone();

    // Agent should be alive immediately
    let alive = registry.get_alive(&agent_id).await;
    assert!(alive.is_ok());

    // Wait for heartbeat to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Agent should now be considered dead
    let alive = registry.get_alive(&agent_id).await;
    assert!(alive.is_err());
}

// ---------------------------------------------------------------------------
// Integration Test 2: Permission Engine + Agent Registry
// End-to-end: Permission checks for registered agents
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_permission_engine_with_registered_agent() {
    let registry: SharedAgentRegistry = create_registry(30);
    let rules = make_test_ruleset();
    let engine = PermissionEngine::new(rules);

    // Register an agent (ID will be UUID)
    let agent = registry
        .register("test-agent".to_string(), None)
        .await
        .unwrap();
    let agent_id = agent.id.clone();

    // Verify file read is allowed (wildcard subject matches any agent)
    let response = engine.evaluate(PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: agent_id.clone(),
        path: "/home/admin/code/main.rs".to_string(),
        op: "read".to_string(),
    }));
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Verify exec is denied (no exec rule)
    let response = engine.evaluate(PermissionRequest::Bare(
        PermissionRequestBody::CommandExec {
            agent: agent_id.clone(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        },
    ));
    assert!(matches!(response, PermissionResponse::Denied { .. }));

    // Verify git status is allowed (has git command rule)
    let response = engine.evaluate(PermissionRequest::Bare(
        PermissionRequestBody::CommandExec {
            agent: agent_id.clone(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        },
    ));
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Verify dangerous git args are denied
    let response = engine.evaluate(PermissionRequest::Bare(
        PermissionRequestBody::CommandExec {
            agent: agent_id.clone(),
            cmd: "git".to_string(),
            args: vec!["reset".to_string(), "--hard".to_string()],
        },
    ));
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_permission_user_and_agent_dual_key_matching() {
    // Build ruleset with dual-key subject
    let ruleset = RuleSetBuilder::new()
        .version("1.0.0")
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

    let engine = PermissionEngine::new(ruleset);

    // Admin user with any agent should get full access (including exec)
    let response = engine.evaluate(PermissionRequest::WithCaller {
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
    });
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Non-admin user should be denied exec
    let response = engine.evaluate(PermissionRequest::WithCaller {
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
    });
    assert!(matches!(response, PermissionResponse::Denied { .. }));

    // But regular user should still have file read permission via agent-only rule
    let response = engine.evaluate(PermissionRequest::WithCaller {
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
    });
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_permission_engine_reload_updates_rules() {
    let initial_rules = RuleSetBuilder::new()
        .version("1.0.0")
        .rule(
            RuleBuilder::new()
                .name("initial-allow")
                .subject_glob("*")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["/tmp/**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let mut engine = PermissionEngine::new(initial_rules);

    // Initial: should allow
    let response = engine.evaluate(PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "any-agent".to_string(),
        path: "/tmp/file.txt".to_string(),
        op: "read".to_string(),
    }));
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Reload with new rules (no rules, default deny)
    let new_rules = RuleSetBuilder::new()
        .version("1.0.0")
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    engine.reload_rules(new_rules);

    // After reload: should deny
    let response = engine.evaluate(PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "any-agent".to_string(),
        path: "/tmp/file.txt".to_string(),
        op: "read".to_string(),
    }));
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

// ---------------------------------------------------------------------------
// Integration Test 3: Config Loading + Agent Registry
// Config-driven agent creation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_config_loading_drives_agent_creation() {
    let registry: SharedAgentRegistry = create_registry(30);

    // Simulate agents.json config
    let json = r#"{
        "version": "1.0.0",
        "agents": [
            {
                "name": "orchestrator",
                "model": "gpt-4",
                "persona": "Master orchestrator",
                "max_iterations": 100
            },
            {
                "name": "builder",
                "model": "claude-3-opus",
                "parent": "orchestrator",
                "persona": "Code builder"
            },
            {
                "name": "tester",
                "model": "claude-3-sonnet",
                "parent": "orchestrator",
                "persona": "Test engineer"
            }
        ]
    }"#;

    let provider = AgentsConfigProvider::from_str(json).unwrap();
    provider.validate().unwrap();

    // Create agents from config
    for agent_config in provider.agents() {
        let _agent = registry
            .register(agent_config.name.clone(), None)
            .await
            .unwrap();
    }

    let agents = registry.list().await;
    assert_eq!(agents.len(), 3);

    let names: Vec<_> = agents.iter().map(|a| a.name.clone()).collect();
    assert!(names.contains(&"orchestrator".to_string()));
    assert!(names.contains(&"builder".to_string()));
    assert!(names.contains(&"tester".to_string()));
}

#[tokio::test]
async fn test_config_validation_rejects_invalid_parent() {
    let json = r#"{
        "version": "1.0.0",
        "agents": [
            {
                "name": "orphan-agent",
                "model": "claude-3-opus",
                "parent": "nonexistent-parent"
            }
        ]
    }"#;

    let provider = AgentsConfigProvider::from_str(json).unwrap();
    let result = provider.validate();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("nonexistent-parent"));
}

#[tokio::test]
async fn test_config_validation_rejects_duplicate_names() {
    let json = r#"{
        "version": "1.0.0",
        "agents": [
            { "name": "agent", "model": "gpt-4" },
            { "name": "agent", "model": "claude-3-opus" }
        ]
    }"#;

    let provider = AgentsConfigProvider::from_str(json).unwrap();
    let result = provider.validate();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Duplicate"));
}

// ---------------------------------------------------------------------------
// Integration Test 4: Skill Loading + Execution Chain
// Skill registry with built-in skills
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_skill_loading_and_execution_chain() {
    // Create skill registry and load built-in skills
    let registry = Arc::new(closeclaw::skills::registry::SkillRegistry::new());

    for skill in closeclaw::skills::builtin::builtin_skills() {
        registry.register(skill).await;
    }

    // Verify skills loaded
    let skills = registry.list().await;
    assert!(skills.contains(&"file_ops".to_string()));
    assert!(skills.contains(&"git_ops".to_string()));
    assert!(skills.contains(&"search".to_string()));
    assert!(skills.contains(&"permission_query".to_string()));
    assert!(skills.contains(&"coding_agent".to_string()));
    assert!(skills.contains(&"skill_creator".to_string()));

    // Get file_ops skill and execute
    let file_ops = registry.get("file_ops").await;
    assert!(file_ops.is_some());
    let file_ops = file_ops.unwrap();

    // Execute file_ops exists method
    let result = file_ops
        .execute("exists", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(value.get("exists").is_some());

    // Execute git_ops status
    let git_ops = registry.get("git_ops").await.unwrap();
    let result = git_ops.execute("status", serde_json::json!({})).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_skill_with_permission_engine_integration() {
    let rules = make_test_ruleset();
    let engine = Arc::new(PermissionEngine::new(rules));

    // Create permission skill with engine reference
    let perm_skill = closeclaw::skills::builtin::PermissionSkill::with_engine(engine.clone());

    // Create a registry for agent
    let registry: SharedAgentRegistry = create_registry(30);
    let agent = registry
        .register("test-agent".to_string(), None)
        .await
        .unwrap();
    let agent_id = agent.id.clone();

    // Query exec permission: should be denied (no exec rule)
    let result = perm_skill
        .execute(
            "query",
            serde_json::json!({
                "agent_id": agent_id,
                "action": "exec"
            }),
        )
        .await;
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("allowed"), Some(&serde_json::json!(false)));
    assert!(value.get("reason").is_some());

    // Query file_read: should be allowed (has wildcard file read rule)
    let result = perm_skill
        .execute(
            "query",
            serde_json::json!({
                "agent_id": agent_id,
                "action": "file_read"
            }),
        )
        .await;
    assert!(result.is_ok());

    // List available actions
    let result = perm_skill
        .execute("list_actions", serde_json::json!({}))
        .await;
    assert!(result.is_ok());
    let value = result.unwrap();
    let actions = value.get("actions").unwrap().as_array().unwrap();
    assert!(actions.contains(&serde_json::json!("exec")));
    assert!(actions.contains(&serde_json::json!("file_read")));
    assert!(actions.contains(&serde_json::json!("file_write")));
}

#[tokio::test]
async fn test_skill_unregister_and_not_found() {
    let registry = Arc::new(closeclaw::skills::registry::SkillRegistry::new());

    for skill in closeclaw::skills::builtin::builtin_skills() {
        registry.register(skill).await;
    }

    // Verify skill exists
    assert!(registry.contains("file_ops").await);

    // Unregister file_ops
    let removed = registry.unregister("file_ops").await;
    assert!(removed);

    // Verify skill gone
    assert!(!registry.contains("file_ops").await);
    assert!(registry.get("file_ops").await.is_none());

    // Unknown skill should return None
    assert!(registry.get("nonexistent").await.is_none());
}

// ---------------------------------------------------------------------------
// Integration Test 5: Permission Engine Template Resolution
// Ensures templates are properly resolved during rule expansion
// ---------------------------------------------------------------------------

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
        .version("1.0.0")
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

    let mut engine = PermissionEngine::new(ruleset);
    engine.load_templates(templates);

    // Test file read via template-resolved rule
    let response = engine.evaluate(PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "any-agent".to_string(),
        path: "/home/admin/code/main.rs".to_string(),
        op: "read".to_string(),
    }));
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Test git status via template-resolved rule
    let response = engine.evaluate(PermissionRequest::Bare(
        PermissionRequestBody::CommandExec {
            agent: "any-agent".to_string(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        },
    ));
    assert!(matches!(response, PermissionResponse::Allowed { .. }));

    // Test unknown command should still be denied (no default allow)
    let response = engine.evaluate(PermissionRequest::Bare(
        PermissionRequestBody::CommandExec {
            agent: "any-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        },
    ));
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}
