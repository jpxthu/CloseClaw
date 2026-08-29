use std::sync::Arc;

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_config::agents::{AgentConfig, ConfigSource, ModelSpec, ResolvedAgentConfig};
use closeclaw_skills::DiskSkillRegistry;

use crate::admin::rpc::protocol::{AdminRequest, AdminResponse, AgentInfoResult};
use crate::admin::rpc::server::{
    dispatch, dispatch_agent_create, dispatch_agent_info, dispatch_agent_list,
    dispatch_skill_install, dispatch_skill_list, reload_registry, AdminContext,
};

fn make_test_context() -> AdminContext {
    let config_dir = tempfile::tempdir().unwrap().keep();
    let config_sub = config_dir.join("config");
    std::fs::create_dir_all(&config_sub).unwrap();
    // agents.json lives in the config subdirectory
    // (ConfigManager.config_dir = config_sub)
    std::fs::write(config_sub.join("agents.json"), r#"{"agents": []}"#).unwrap();
    let config_manager = Arc::new(closeclaw_config::ConfigManager::new(config_sub).unwrap());
    AdminContext {
        agent_registry: Arc::new(AgentRegistry::new()),
        skill_registry: Arc::new(std::sync::RwLock::new(Some(DiskSkillRegistry::default()))),
        config_manager,
        config_dir,
        restart_tx: None,
    }
}

#[test]
fn test_dispatch_agent_list_empty() {
    let ctx = make_test_context();
    let resp = dispatch_agent_list(&ctx);
    match resp {
        AdminResponse::AgentListResult { agents } => assert!(agents.is_empty()),
        _ => panic!("expected AgentListResult"),
    }
}

#[test]
fn test_dispatch_agent_info_not_found() {
    let ctx = make_test_context();
    let resp = dispatch_agent_info("nonexistent", &ctx);
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("not found"));
        }
        _ => panic!("expected Error"),
    }
}

#[tokio::test]
async fn test_dispatch_skill_list_empty() {
    let ctx = make_test_context();
    let resp = dispatch_skill_list(&ctx).await;
    match resp {
        AdminResponse::SkillListResult { skills } => assert!(skills.is_empty()),
        _ => panic!("expected SkillListResult"),
    }
}

#[tokio::test]
async fn test_dispatch_skill_install_not_found() {
    let ctx = make_test_context();
    let resp = dispatch_skill_install("test-skill", &ctx).await;
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("not found"));
        }
        _ => panic!("expected Error for missing skill"),
    }
}

#[tokio::test]
async fn test_dispatch_agent_create_empty_name() {
    let ctx = make_test_context();
    let resp = dispatch_agent_create("", Some("gpt-4".into()), &ctx).await;
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("empty"));
        }
        _ => panic!("expected Error for empty name"),
    }
}

#[tokio::test]
async fn test_dispatch_agent_create_success() {
    let ctx = make_test_context();
    let resp = dispatch_agent_create("test-agent", Some("gpt-4".into()), &ctx).await;
    assert!(matches!(resp, AdminResponse::Ok));
    // Verify agent was created
    assert!(ctx.agent_registry.get("test-agent").is_some());
}

#[tokio::test]
async fn test_dispatch_agent_create_duplicate() {
    let ctx = make_test_context();
    let resp = dispatch_agent_create("dup-agent", None, &ctx).await;
    assert!(matches!(resp, AdminResponse::Ok));
    // Try creating same agent again
    let resp = dispatch_agent_create("dup-agent", None, &ctx).await;
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("already exists"));
        }
        _ => panic!("expected Error for duplicate"),
    }
}

#[tokio::test]
async fn test_dispatch_ping() {
    let ctx = make_test_context();
    let resp = dispatch(AdminRequest::Ping, &ctx).await;
    assert!(matches!(resp, AdminResponse::Pong));
}

// ═══════════════════════════════════════════════════════════════════════════
// reload_registry tests — verify full-replacement (clear + insert) semantics
// ═══════════════════════════════════════════════════════════════════════════

/// Write agent config files and agents.json for testing reload.
///
/// Directory layout:
///   config_dir/
///   ├── config/
///   │   └── agents.json        (list of agent IDs)
///   └── agents/
///       ├── <id1>/config.json
///       └── <id2>/config.json
fn setup_agents(config_dir: &std::path::Path, ids: &[&str]) {
    let config_sub = config_dir.join("config");
    let agents_dir = config_dir.join("agents");
    std::fs::create_dir_all(&config_sub).unwrap();
    std::fs::create_dir_all(&agents_dir).unwrap();

    // Write agents.json
    let agent_ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    let json = serde_json::json!({ "agents": agent_ids });
    std::fs::write(
        config_sub.join("agents.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    // Write each agent's config.json
    for id in ids {
        let agent_dir = agents_dir.join(id);
        std::fs::create_dir_all(&agent_dir).unwrap();
        let config = AgentConfig {
            id: id.to_string(),
            ..AgentConfig::default()
        };
        std::fs::write(
            agent_dir.join("config.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
    }
}

/// Create a ConfigManager pointed at the given config_dir ("config" subdir)
/// and call load_agents so it discovers the agent files on disk.
fn make_context_with_agents(config_dir: &std::path::Path) -> AdminContext {
    let config_sub = config_dir.join("config");
    let config_manager = closeclaw_config::ConfigManager::new(config_sub.clone()).unwrap();
    // Load agents from the filesystem so config_manager.agents() is populated.
    config_manager.load_agents(None).unwrap();
    AdminContext {
        agent_registry: Arc::new(AgentRegistry::new()),
        skill_registry: Arc::new(std::sync::RwLock::new(Some(DiskSkillRegistry::default()))),
        config_manager: Arc::new(config_manager),
        config_dir: config_dir.to_path_buf(),
        restart_tx: None,
    }
}

/// Normal path: registry has agents A and B → reload with only C → only C
/// remains, A/B are removed.
#[test]
fn test_reload_full_replacement() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path();

    // Step 1: set up config with agents A and B
    setup_agents(config_dir, &["agent-a", "agent-b"]);
    let ctx = make_context_with_agents(config_dir);

    // Populate registry with A and B (simulates startup)
    let startup_configs: Vec<_> = ctx.config_manager.agents().into_values().collect();
    assert_eq!(startup_configs.len(), 2);
    ctx.agent_registry.populate(startup_configs);
    assert!(ctx.agent_registry.get("agent-a").is_some());
    assert!(ctx.agent_registry.get("agent-b").is_some());

    // Step 2: change config to only agent C
    // Remove A and B config files and agents.json entries
    setup_agents(config_dir, &["agent-c"]);

    // Step 3: reload — should clear A/B and insert C
    let result = reload_registry(&ctx);
    assert!(result.is_ok());

    assert!(
        ctx.agent_registry.get("agent-a").is_none(),
        "agent-a should be removed after reload"
    );
    assert!(
        ctx.agent_registry.get("agent-b").is_none(),
        "agent-b should be removed after reload"
    );
    assert!(
        ctx.agent_registry.get("agent-c").is_some(),
        "agent-c should be present after reload"
    );
}

/// Boundary: reload with empty config list → registry completely cleared.
#[test]
fn test_reload_empty_config_clears_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path();

    // Set up config with agents A and B, populate registry
    setup_agents(config_dir, &["agent-a", "agent-b"]);
    let ctx = make_context_with_agents(config_dir);
    let startup_configs: Vec<_> = ctx.config_manager.agents().into_values().collect();
    ctx.agent_registry.populate(startup_configs);
    assert_eq!(ctx.agent_registry.iter().count(), 2);

    // Change config to empty agent list
    setup_agents(config_dir, &[]);

    // Reload should clear the registry
    let result = reload_registry(&ctx);
    assert!(result.is_ok());
    assert_eq!(
        ctx.agent_registry.iter().count(),
        0,
        "registry should be empty after reload with empty config"
    );
}

/// State transition: populate at startup → reload at runtime → old data
/// does not leak.
#[test]
fn test_populate_then_reload_no_stale_data() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path();

    // Phase 1: startup — populate with agents A, B, C
    setup_agents(config_dir, &["agent-a", "agent-b", "agent-c"]);
    let ctx = make_context_with_agents(config_dir);
    let startup_configs: Vec<_> = ctx.config_manager.agents().into_values().collect();
    assert_eq!(startup_configs.len(), 3);
    ctx.agent_registry.populate(startup_configs);
    assert!(ctx.agent_registry.get("agent-a").is_some());
    assert!(ctx.agent_registry.get("agent-b").is_some());
    assert!(ctx.agent_registry.get("agent-c").is_some());

    // Phase 2: runtime — config changes to only D and E
    setup_agents(config_dir, &["agent-d", "agent-e"]);
    let result = reload_registry(&ctx);
    assert!(result.is_ok());

    // Verify old agents are gone
    assert!(
        ctx.agent_registry.get("agent-a").is_none(),
        "agent-a from startup should not persist after reload"
    );
    assert!(
        ctx.agent_registry.get("agent-b").is_none(),
        "agent-b from startup should not persist after reload"
    );
    assert!(
        ctx.agent_registry.get("agent-c").is_none(),
        "agent-c from startup should not persist after reload"
    );

    // Verify new agents are present
    assert!(
        ctx.agent_registry.get("agent-d").is_some(),
        "agent-d should be present after reload"
    );
    assert!(
        ctx.agent_registry.get("agent-e").is_some(),
        "agent-e should be present after reload"
    );
    assert_eq!(
        ctx.agent_registry.iter().count(),
        2,
        "registry should contain exactly 2 agents after reload"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.3 — agent info behavior dimension tests
// ═══════════════════════════════════════════════════════════════════════════

fn make_full_config() -> ResolvedAgentConfig {
    #[allow(deprecated)] // default_child_agent is deprecated; test includes it for completeness
    let cfg = AgentConfig {
        id: "full-agent".to_string(),
        name: Some("Full Agent".to_string()),
        parent_id: Some("parent-1".to_string()),
        model: Some(ModelSpec::single("gpt-4o")),
        workspace: Some("/workspace/full".to_string()),
        agent_dir: Some("/agent/dir/full".to_string()),
        bootstrap_mode: Some(closeclaw_common::BootstrapMode::Minimal),
        skills: vec!["skill-a".to_string(), "skill-b".to_string()],
        tools: vec!["read".to_string(), "write".to_string()],
        disallowed_tools: vec!["dangerous".to_string()],
        subagents: closeclaw_config::agents::SubagentsConfig {
            allow_agents: vec!["child-a".to_string()],
            require_agent_id: Some(true),
            max_spawn_depth: Some(3),
            max_children: Some(10),
            timeout: Some(120),
            timeout_warning: Some(60),
            timeout_notify_interval_ratio: Some(0.5),
            default_child_agent: None,
            model: Some(ModelSpec::single("gpt-4o-mini")),
        },
        memory: Some(closeclaw_config::agents::MemoryConfig::default()),
        ..AgentConfig::default()
    };
    ResolvedAgentConfig::from_single(cfg, ConfigSource::User, "test", None).unwrap()
}

/// Helper: build a ResolvedAgentConfig with all fields at defaults.
fn make_default_config() -> ResolvedAgentConfig {
    let cfg = AgentConfig {
        id: "default-agent".to_string(),
        ..AgentConfig::default()
    };
    ResolvedAgentConfig::from_single(cfg, ConfigSource::User, "test", None).unwrap()
}

/// Populate the test context's registry with the given configs.
fn populate_registry(ctx: &AdminContext, configs: Vec<ResolvedAgentConfig>) {
    ctx.agent_registry.populate(configs);
}

/// Normal path: fully configured agent -> all 12 fields present and correct.
#[test]
fn test_dispatch_agent_info_full_fields() {
    let ctx = make_test_context();
    populate_registry(&ctx, vec![make_full_config()]);

    let resp = dispatch_agent_info("full-agent", &ctx);
    match resp {
        AdminResponse::AgentInfoResult(info) => {
            assert_eq!(info.id, "full-agent");
            assert_eq!(info.name, "Full Agent");
            assert_eq!(info.parent_id.as_deref(), Some("parent-1"));
            assert_eq!(
                info.model.as_ref().map(|m| m.primary.as_str()),
                Some("gpt-4o")
            );
            assert_eq!(info.workspace.as_deref(), Some("/workspace/full"));
            assert_eq!(info.agent_dir.as_deref(), Some("/agent/dir/full"));
            assert_eq!(
                info.bootstrap_mode,
                closeclaw_common::BootstrapMode::Minimal
            );
            assert_eq!(info.skills, vec!["skill-a", "skill-b"]);
            assert_eq!(info.tools, vec!["read", "write"]);
            assert_eq!(info.disallowed_tools, vec!["dangerous"]);
            assert_eq!(info.subagents.max_spawn_depth, Some(3));
            assert_eq!(info.subagents.max_children, Some(10));
            assert!(info.memory.is_some());
        }
        other => panic!("expected AgentInfoResult, got {:?}", other),
    }
}

/// Default values: all-default config -> fields use defaults.
#[test]
fn test_dispatch_agent_info_default_values() {
    let ctx = make_test_context();
    populate_registry(&ctx, vec![make_default_config()]);

    let resp = dispatch_agent_info("default-agent", &ctx);
    match resp {
        AdminResponse::AgentInfoResult(info) => {
            assert_eq!(info.id, "default-agent");
            assert_eq!(info.name, "default-agent");
            assert!(info.parent_id.is_none());
            assert!(info.model.is_none());
            assert!(info.workspace.is_none());
            assert!(info.agent_dir.is_none());
            assert_eq!(info.bootstrap_mode, closeclaw_common::BootstrapMode::Full);
            // AgentConfig::default() uses default_all() -> ["*"]
            assert_eq!(info.skills, vec!["*"]);
            assert_eq!(info.tools, vec!["*"]);
            assert!(info.disallowed_tools.is_empty());
            assert_eq!(info.subagents.max_spawn_depth, Some(1));
            assert_eq!(info.subagents.max_children, Some(5));
            assert!(
                info.memory.is_none(),
                "default config should have memory=null"
            );
        }
        other => panic!("expected AgentInfoResult, got {:?}", other),
    }
}

/// Error path: nonexistent id -> Error message contains the id.
#[test]
fn test_dispatch_agent_info_error_contains_id() {
    let ctx = make_test_context();
    let resp = dispatch_agent_info("missing-agent", &ctx);
    match resp {
        AdminResponse::Error { message } => {
            assert!(
                message.contains("missing-agent"),
                "error should contain the id: {}",
                message
            );
            assert!(message.contains("not found"));
        }
        other => panic!("expected Error, got {:?}", other),
    }
}

/// Query by name (not id) should not match - verifies id semantics.
#[test]
fn test_dispatch_agent_info_name_not_queried() {
    let ctx = make_test_context();
    let cfg = AgentConfig {
        id: "actual-id".to_string(),
        name: Some("Display Name".to_string()),
        ..AgentConfig::default()
    };
    let resolved = ResolvedAgentConfig::from_single(cfg, ConfigSource::User, "test", None).unwrap();
    populate_registry(&ctx, vec![resolved]);

    // Querying by the display name should fail
    let resp = dispatch_agent_info("Display Name", &ctx);
    match resp {
        AdminResponse::Error { message } => {
            assert!(message.contains("Display Name"));
            assert!(message.contains("not found"));
        }
        other => panic!("expected Error for name query, got {:?}", other),
    }
    // Querying by the actual id should succeed
    let resp = dispatch_agent_info("actual-id", &ctx);
    assert!(matches!(resp, AdminResponse::AgentInfoResult(_)));
}

/// Protocol roundtrip: AgentInfoResult serde field names are camelCase.
#[test]
fn test_agent_info_result_camelcase_fields() {
    use closeclaw_config::agents::SubagentsConfig;
    let info = AgentInfoResult {
        id: "test".to_string(),
        name: "Test".to_string(),
        parent_id: Some("p".to_string()),
        model: Some(ModelSpec::single("gpt-4o")),
        workspace: Some("/ws".to_string()),
        agent_dir: Some("/ad".to_string()),
        bootstrap_mode: closeclaw_common::BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        memory: None,
    };
    let resp = AdminResponse::AgentInfoResult(Box::new(info));
    let value = serde_json::to_value(&resp).unwrap();

    // Tagged enum flattens struct fields; type is at top level
    assert_eq!(value["type"], "agent_info_result");

    // Verify camelCase field names match agent-config.md
    assert!(value.get("id").is_some(), "missing 'id' field");
    assert!(value.get("name").is_some(), "missing 'name' field");
    assert!(value.get("parentId").is_some(), "missing 'parentId' field");
    assert!(value.get("model").is_some(), "missing 'model' field");
    assert!(
        value.get("workspace").is_some(),
        "missing 'workspace' field"
    );
    assert!(value.get("agentDir").is_some(), "missing 'agentDir' field");
    assert!(
        value.get("bootstrapMode").is_some(),
        "missing 'bootstrapMode' field"
    );
    assert!(value.get("skills").is_some(), "missing 'skills' field");
    assert!(value.get("tools").is_some(), "missing 'tools' field");
    assert!(
        value.get("disallowedTools").is_some(),
        "missing 'disallowedTools' field"
    );
    assert!(
        value.get("subagents").is_some(),
        "missing 'subagents' field"
    );
    assert!(value.get("memory").is_some(), "missing 'memory' field");
}
