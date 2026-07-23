//! Integration Tests for CloseClaw
//!
//! Tests in this file verify cross-module interactions:
//! - Agent registry config query
//! - Config loading + agent registry
//! - Skill loading + execution chain

use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test imports — from the library crate
// ---------------------------------------------------------------------------
use closeclaw::agent::config::AgentConfig;
use closeclaw::agent::registry::{create_registry, SharedAgentRegistry};
use closeclaw_config::agents::{AgentsConfigProvider, ConfigSource, ResolvedAgentConfig};
use closeclaw_permission::engine::{Action, CommandArgs, Effect, PermissionEngine, RuleSet};
use closeclaw_permission::rules::{RuleBuilder, RuleSetBuilder};
use closeclaw_skills::Skill;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Helper builders
// ---------------------------------------------------------------------------

/// Creates a ruleset that grants permissions to any agent (using wildcard patterns).
/// This allows testing permission engine + agent registry integration without
/// having to worry about UUID vs name mismatches.
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
        .default_file_read(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Integration Test 1: Agent Registry Config Query
// Tests populate + get for config storage and parent_id field query
// ---------------------------------------------------------------------------

/// Creates a [`ResolvedAgentConfig`] for testing purposes.
///
/// Given an agent `id` and an optional `parent_id`, builds a default
/// [`AgentConfig`], wraps it with [`ConfigSource::User`], and returns
/// a fully resolved configuration suitable for [`SharedAgentRegistry::populate`].
fn make_resolved_config(id: &str, parent_id: Option<&str>) -> ResolvedAgentConfig {
    let cfg = AgentConfig {
        id: id.to_string(),
        parent_id: parent_id.map(String::from),
        ..Default::default()
    };
    ResolvedAgentConfig::from_single(cfg, ConfigSource::User, "<test>", None).unwrap()
}

#[tokio::test]
async fn test_agent_registry_config_query() {
    let registry: SharedAgentRegistry = create_registry();

    // Build configs with parent-child relationships via parent_id
    let configs = vec![
        make_resolved_config("parent-agent", None),
        make_resolved_config("child-agent", Some("parent-agent")),
        make_resolved_config("grandchild-agent", Some("child-agent")),
    ];

    // Populate registry with all configs
    registry.populate(configs);

    // Verify each config is stored correctly
    let parent = registry.get("parent-agent");
    assert!(parent.is_some());
    let parent = parent.unwrap();
    assert_eq!(parent.id, "parent-agent");
    assert!(parent.parent_id.is_none());

    let child = registry.get("child-agent");
    assert!(child.is_some());
    let child = child.unwrap();
    assert_eq!(child.id, "child-agent");
    assert_eq!(child.parent_id.as_deref(), Some("parent-agent"));

    let grandchild = registry.get("grandchild-agent");
    assert!(grandchild.is_some());
    let grandchild = grandchild.unwrap();
    assert_eq!(grandchild.id, "grandchild-agent");
    assert_eq!(grandchild.parent_id.as_deref(), Some("child-agent"));

    // Verify non-existent agent returns None
    assert!(registry.get("unknown-agent").is_none());
}

// ---------------------------------------------------------------------------
// Integration Test 3: Config Loading + Agent Registry
// Config-driven agent creation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_config_loading_drives_agent_creation() {
    let registry: SharedAgentRegistry = create_registry();

    // Simulate agents.json config (registration list of agent IDs)
    let json = r#"{
        "agents": ["orchestrator", "builder", "tester"]
    }"#;

    let provider = AgentsConfigProvider::from_json_str(json).unwrap();
    provider.validate().unwrap();

    // Create ResolvedAgentConfig from provider and populate registry
    let configs: Vec<_> = provider
        .agents()
        .iter()
        .map(|id| make_resolved_config(id, None))
        .collect();
    registry.populate(configs);

    // Verify each agent is stored correctly via get
    for agent_id in provider.agents() {
        let agent = registry.get(agent_id);
        assert!(
            agent.is_some(),
            "agent '{}' should exist in registry",
            agent_id
        );
        assert_eq!(agent.unwrap().id, *agent_id);
    }
}

#[tokio::test]
async fn test_config_validation_rejects_duplicate_ids() {
    let json = r#"{
        "agents": ["agent", "agent"]
    }"#;

    let provider = AgentsConfigProvider::from_json_str(json).unwrap();
    let result = provider.validate();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Duplicate"));
}

#[tokio::test]
async fn test_config_validation_rejects_empty_id() {
    let json = r#"{
        "agents": [""]
    }"#;

    let provider = AgentsConfigProvider::from_json_str(json).unwrap();
    let result = provider.validate();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("empty"));
}

// ---------------------------------------------------------------------------
// Integration Test 4: Skill Loading + Execution Chain
// Skill registry with built-in skills
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_skill_loading_and_execution_chain() {
    // Create skill registry and load built-in skills
    let registry = Arc::new(closeclaw_skills::registry::BuiltinSkillRegistry::new());

    for skill in closeclaw_skills::builtin::builtin_skills() {
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
        .execute(
            "exists",
            serde_json::json!({"path": "Cargo.toml", "agent_id": "test-agent"}),
        )
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
    let engine = Arc::new(RwLock::new(PermissionEngine::new_with_default_data_root(
        rules,
    )));

    // Create permission skill with engine reference
    let perm_skill = closeclaw_skills::builtin::PermissionSkill::with_engine(engine.clone());

    // Populate registry with a test agent config
    let registry: SharedAgentRegistry = create_registry();
    let configs = vec![make_resolved_config("test-agent", None)];
    registry.populate(configs);

    let agent = registry.get("test-agent").unwrap();
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
    assert!(actions.contains(&serde_json::json!("command")));
    assert!(actions.contains(&serde_json::json!("file_read")));
    assert!(actions.contains(&serde_json::json!("file_write")));
}

#[tokio::test]
async fn test_skill_unregister_and_not_found() {
    let registry = Arc::new(closeclaw_skills::registry::BuiltinSkillRegistry::new());

    for skill in closeclaw_skills::builtin::builtin_skills() {
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

// Permission template resolution test moved to integration_permission_tests.rs
