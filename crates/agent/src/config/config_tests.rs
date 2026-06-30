use std::collections::HashMap;

use super::*;
use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};
use tempfile::TempDir;
#[test]
fn test_agent_config_save_load() {
    let temp = TempDir::new().unwrap();
    let config = AgentConfig {
        id: "test-id".to_string(),
        name: Some("Test Agent".to_string()),
        parent_id: Some("parent-id".to_string()),
        ..Default::default()
    };

    let path = temp.path().join("config.json");
    config.save(&path).unwrap();
    let loaded = AgentConfig::load(&path).unwrap();

    assert_eq!(loaded.id, config.id);
    assert_eq!(loaded.name, config.name);
    assert_eq!(loaded.parent_id, config.parent_id);
}

#[test]
fn test_permissions_save_load() {
    let temp = TempDir::new().unwrap();
    let mut permissions = AgentPermissions {
        agent_id: "test-id".to_string(),
        permissions: HashMap::new(),
        inherited_from: Some("parent-id".to_string()),
    };
    permissions.permissions.insert(
        "exec".to_string(),
        ActionPermission {
            allowed: true,
            limits: PermissionLimits {
                commands: vec!["/usr/bin/git".to_string()],
                paths: vec![],
                timeout_ms: Some(300000),
            },
        },
    );

    let path = temp.path().join("permissions.json");
    permissions.save(&path).unwrap();
    let loaded = AgentPermissions::load(&path).unwrap();

    assert_eq!(loaded.agent_id, permissions.agent_id);
    assert!(loaded.is_allowed("exec"));
    assert!(!loaded.is_allowed("network"));
}

#[test]
fn test_default_communication_config() {
    // CommunicationConfig is still available as a standalone type.
    let with_parent = CommunicationConfig::default_with_parent(Some("parent-1"));
    assert_eq!(with_parent.outbound, vec!["parent-1"]);
    assert_eq!(with_parent.inbound, vec!["parent-1"]);

    let without_parent = CommunicationConfig::default_with_parent(None);
    assert!(without_parent.outbound.is_empty());
    assert!(without_parent.inbound.is_empty());
}

#[test]
fn test_communication_allowed() {
    // CommunicationConfig and check_communication_allowed still work as
    // standalone functions, even though AgentConfig no longer has a
    // communication field (removed in Step 1.4 - not in design doc).
    let source_comm = CommunicationConfig {
        outbound: vec!["child-1".to_string()],
        inbound: vec!["child-1".to_string()],
    };

    let target_comm = CommunicationConfig::default_with_parent(Some("parent-1"));

    // Parent -> Child should be allowed
    let result = check_communication_allowed(&source_comm, "parent-1", &target_comm, "child-1");
    assert_eq!(result, CommunicationCheckResult::Allowed);

    // Child -> Parent should be allowed
    let result = check_communication_allowed(&target_comm, "child-1", &source_comm, "parent-1");
    assert_eq!(result, CommunicationCheckResult::Allowed);
}

#[test]
fn test_communication_denied_outbound() {
    let agent_a_comm = CommunicationConfig {
        outbound: vec!["agent-b".to_string()],
        inbound: vec!["agent-b".to_string()],
    };

    let agent_c_comm = CommunicationConfig {
        outbound: vec![],
        inbound: vec![],
    };

    // Agent A -> Agent C: A's outbound doesn't contain C
    let result = check_communication_allowed(&agent_a_comm, "agent-a", &agent_c_comm, "agent-c");
    assert_eq!(result, CommunicationCheckResult::TargetNotInSourceOutbound);
}

#[test]
fn test_communication_denied_inbound() {
    let agent_a_comm = CommunicationConfig {
        outbound: vec!["agent-b".to_string()],
        inbound: vec!["agent-b".to_string()],
    };

    let agent_b_comm = CommunicationConfig {
        outbound: vec![],
        inbound: vec![], // B doesn't accept inbound from anyone
    };

    // Agent A -> Agent B: A's outbound contains B, but B's inbound doesn't contain A
    let result = check_communication_allowed(&agent_a_comm, "agent-a", &agent_b_comm, "agent-b");
    assert_eq!(result, CommunicationCheckResult::SourceNotInTargetInbound);
}

#[test]
fn test_agent_config_new_fields_defaults() {
    let config = AgentConfig::default();

    // New fields should have sensible defaults
    assert!(config.model.is_none());
    assert!(config.workspace.is_none());
    assert!(config.agent_dir.is_none());
    assert_eq!(config.bootstrap_mode, BootstrapMode::Full);
    assert_eq!(config.skills, vec!["*"]);
    assert_eq!(config.tools, vec!["*"]);
    assert!(config.disallowed_tools.is_empty());
}

#[test]
fn test_agent_config_new_fields_roundtrip() {
    let temp = TempDir::new().unwrap();
    let workspace_path = temp.path().join("workspace");
    let agent_dir_path = temp.path().join("agent_dir");
    let config = AgentConfig {
        id: "test-agent".to_string(),
        name: Some("Test".to_string()),
        model: Some("gpt-4o".to_string()),
        workspace: Some(workspace_path.to_str().unwrap().to_string()),
        agent_dir: Some(agent_dir_path.to_str().unwrap().to_string()),
        bootstrap_mode: BootstrapMode::Minimal,
        skills: vec!["skill-a".to_string(), "skill-b".to_string()],
        tools: vec!["read".to_string(), "write".to_string()],
        disallowed_tools: vec!["exec".to_string()],
        subagents: SubagentsConfig {
            allow_agents: vec!["agent-1".to_string()],
            require_agent_id: true,
            max_spawn_depth: 3,
            max_children: 10,
            default_child_agent: Some("child-agent".to_string()),
            model: Some("claude-3".to_string()),
        },
        ..Default::default()
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: AgentConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.model, Some("gpt-4o".to_string()));
    assert_eq!(
        deserialized.workspace,
        Some(workspace_path.to_str().unwrap().to_string())
    );
    assert_eq!(
        deserialized.agent_dir,
        Some(agent_dir_path.to_str().unwrap().to_string())
    );
    assert_eq!(deserialized.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(deserialized.skills, vec!["skill-a", "skill-b"]);
    assert_eq!(deserialized.tools, vec!["read", "write"]);
    assert_eq!(deserialized.disallowed_tools, vec!["exec"]);
    assert_eq!(deserialized.subagents.allow_agents, vec!["agent-1"]);
    assert!(deserialized.subagents.require_agent_id);
    assert_eq!(deserialized.subagents.max_spawn_depth, 3);
    assert_eq!(deserialized.subagents.max_children, 10);
    assert_eq!(
        deserialized.subagents.default_child_agent,
        Some("child-agent".to_string())
    );
    assert_eq!(deserialized.subagents.model, Some("claude-3".to_string()));
}

#[test]
fn test_subagents_config_defaults() {
    let config = SubagentsConfig::default();

    assert_eq!(config.allow_agents, vec!["*"]);
    assert!(!config.require_agent_id);
    assert_eq!(config.max_spawn_depth, 1);
    assert_eq!(config.max_children, 5);
    assert!(config.default_child_agent.is_none());
    assert!(config.model.is_none());
}

#[test]
fn test_agent_config_json_with_new_fields() {
    let json = r#"{
        "id": "from-json",
        "name": "Json Agent",
        "model": "deepseek-v3",
        "workspace": "/home/user/project",
        "agentDir": "/home/user/.agents/my-agent",
        "bootstrapMode": "minimal",
        "skills": ["coding", "search"],
        "tools": ["read", "write", "exec"],
        "disallowedTools": ["web_search"],
        "subagents": {
            "allowAgents": ["coding-agent", "review-agent"],
            "requireAgentId": true,
            "maxSpawnDepth": 2,
            "maxChildren": 8,
            "defaultChildAgent": "coding-agent",
            "model": "gpt-4o-mini"
        }
    }"#;

    let config: AgentConfig = serde_json::from_str(json).unwrap();

    assert_eq!(config.id, "from-json");
    assert_eq!(config.name, Some("Json Agent".to_string()));
    assert_eq!(config.model, Some("deepseek-v3".to_string()));
    assert_eq!(config.workspace, Some("/home/user/project".to_string()));
    assert_eq!(
        config.agent_dir,
        Some("/home/user/.agents/my-agent".to_string())
    );
    assert_eq!(config.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(config.skills, vec!["coding", "search"]);
    assert_eq!(config.tools, vec!["read", "write", "exec"]);
    assert_eq!(config.disallowed_tools, vec!["web_search"]);
    assert_eq!(
        config.subagents.allow_agents,
        vec!["coding-agent", "review-agent"]
    );
    assert!(config.subagents.require_agent_id);
    assert_eq!(config.subagents.max_spawn_depth, 2);
    assert_eq!(config.subagents.max_children, 8);
    assert_eq!(
        config.subagents.default_child_agent,
        Some("coding-agent".to_string())
    );
    assert_eq!(config.subagents.model, Some("gpt-4o-mini".to_string()));
}

#[test]
fn test_agent_config_camel_case_parse() {
    // JSON 示例直接取自设计文档 docs/design/agent/agent-config.md，
    // 验证用户按设计文档编写的 camelCase 配置能被正确解析。
    let json = r#"{
        "id": "code-reviewer",
        "name": "代码审查助手",
        "parentId": null,
        "model": "deepseek/deepseek-chat",
        "workspace": null,
        "agentDir": null,
        "bootstrapMode": "minimal",
        "skills": ["code-review"],
        "tools": ["read", "grep", "glob", "web_search", "web_fetch"],
        "disallowedTools": [],
        "subagents": {
            "allowAgents": [],
            "maxSpawnDepth": 0,
            "maxChildren": 0
        }
    }"#;

    let config: AgentConfig = serde_json::from_str(json).unwrap();

    assert_eq!(config.id, "code-reviewer");
    assert_eq!(config.name, Some("代码审查助手".to_string()));
    assert_eq!(config.parent_id, None);
    assert_eq!(config.model, Some("deepseek/deepseek-chat".to_string()));
    assert_eq!(config.workspace, None);
    assert_eq!(config.agent_dir, None);
    assert_eq!(config.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(config.skills, vec!["code-review"]);
    assert_eq!(
        config.tools,
        vec!["read", "grep", "glob", "web_search", "web_fetch"]
    );
    assert!(config.disallowed_tools.is_empty());
    assert!(config.subagents.allow_agents.is_empty());
    assert!(!config.subagents.require_agent_id);
    assert_eq!(config.subagents.max_spawn_depth, 0);
    assert_eq!(config.subagents.max_children, 0);
    assert_eq!(config.subagents.default_child_agent, None);
    assert_eq!(config.subagents.model, None);
}

// Step 1.5 — Tests for optional name and id fallback

#[test]
fn test_agent_config_name_defaults_to_none() {
    // Minimal JSON with only `id`: `name` must default to `None`.
    let json = r#"{"id": "x"}"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.id, "x");
    assert_eq!(config.name, None);
}

#[test]
fn test_agent_config_name_empty_string_fallback() {
    // JSON with `name: ""` is preserved as `Some("")` at the deserialization
    // layer. The empty-string → id fallback happens later in
    // `ResolvedAgentConfig::from_single` / `merge`, not in serde.
    let json = r#"{"id": "x", "name": ""}"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.id, "x");
    assert_eq!(config.name, Some("".to_string()));
}

#[test]
fn test_resolved_config_name_fallback_to_id() {
    // `name = None` → resolved `name` must equal `id`.
    let config = AgentConfig {
        id: "agent-x".to_string(),
        name: None,
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>").unwrap();
    assert_eq!(resolved.id, "agent-x");
    assert_eq!(resolved.name, "agent-x");
}

#[test]
fn test_resolved_config_name_empty_string_fallback() {
    // `name = Some("")` → resolved `name` must equal `id`.
    let config = AgentConfig {
        id: "agent-y".to_string(),
        name: Some("".to_string()),
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>").unwrap();
    assert_eq!(resolved.id, "agent-y");
    assert_eq!(resolved.name, "agent-y");
}

#[test]
fn test_resolved_config_merge_name_fallback() {
    // Both project and user have no usable name → merged name falls back to
    // the resolved `id`. Project.name is `None`, user.name is `Some("")`.
    let project = AgentConfig {
        id: "agent-z".to_string(),
        name: None,
        ..Default::default()
    };
    let user = AgentConfig {
        id: "agent-z".to_string(),
        name: Some("".to_string()),
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(resolved.id, "agent-z");
    assert_eq!(resolved.name, "agent-z");
    assert_eq!(resolved.source, ConfigSource::Merged);
}

// Step 1.2–1.4 — Reverse serde: unknown fields from removed fields are
// silently ignored. This ensures backward compatibility with old config
// files that still contain deprecated fields.

#[test]
fn test_agent_config_ignores_removed_fields() {
    // JSON contains all fields removed in Steps 1.2–1.4:
    // - max_child_depth (Step 1.2)
    // - state, created_at (Step 1.3)
    // - communication, wait_timeout_secs, grace_period_secs (Step 1.4)
    let json = r#"{
        "id": "legacy-agent",
        "name": "Legacy Agent",
        "max_child_depth": 5,
        "created_at": "2025-01-15T10:30:00Z",
        "state": "running",
        "communication": {
            "outbound": ["child-1"],
            "inbound": ["parent-1"]
        },
        "wait_timeout_secs": 30,
        "grace_period_secs": 10,
        "model": "gpt-4o"
    }"#;

    // Must deserialize without error — unknown fields are ignored.
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.id, "legacy-agent");
    assert_eq!(config.name, Some("Legacy Agent".to_string()));
    assert_eq!(config.model, Some("gpt-4o".to_string()));

    // Ensure removed fields are NOT present on the struct.
    // (Compile-time check: these fields don't exist on AgentConfig.)
    let json_str = serde_json::to_string(&config).unwrap();
    let reparsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!(
        reparsed.get("max_child_depth").is_none(),
        "max_child_depth should not be serialized"
    );
    assert!(
        reparsed.get("state").is_none(),
        "state should not be serialized"
    );
    assert!(
        reparsed.get("createdAt").is_none(),
        "createdAt should not be serialized"
    );
    assert!(
        reparsed.get("communication").is_none(),
        "communication should not be serialized"
    );
    assert!(
        reparsed.get("waitTimeoutSecs").is_none(),
        "waitTimeoutSecs should not be serialized"
    );
    assert!(
        reparsed.get("gracePeriodSecs").is_none(),
        "gracePeriodSecs should not be serialized"
    );
}

// --- Step 1.3: Tests for design-doc alignment ---

#[test]
fn test_merge_skills_star_overrides_user() {
    // Project-level ["*"] should override user-level specific list.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        skills: vec!["*".to_string()],
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        skills: vec!["specific-skill".to_string()],
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(
        resolved.skills,
        vec!["*".to_string()],
        "project-level [\"*\"] should override user-level skills"
    );
}

#[test]
fn test_merge_tools_star_overrides_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        tools: vec!["*".to_string()],
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        tools: vec!["read".to_string(), "grep".to_string()],
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(
        resolved.tools,
        vec!["*".to_string()],
        "project-level [\"*\"] should override user-level tools"
    );
}

#[test]
fn test_merge_allow_agents_star_overrides_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            allow_agents: vec!["*".to_string()],
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            allow_agents: vec!["agent-a".to_string()],
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(
        resolved.subagents.allow_agents,
        vec!["*".to_string()],
        "project-level [\"*\"] should override user-level allow_agents"
    );
}

#[test]
fn test_merge_skills_empty_project_falls_back_to_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        skills: vec![],
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        skills: vec!["user-skill".to_string()],
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(
        resolved.skills,
        vec!["user-skill".to_string()],
        "empty project skills should fall back to user skills"
    );
}

#[test]
fn test_agent_config_deserialize_ignores_permissions_key() {
    // After design-doc alignment, AgentConfig should not have a permissions field.
    // Deserializing JSON with a permissions field should not populate any such field.
    let json = r#"{
        "id": "no-inline-perms",
        "permissions": {
            "agent_id": "no-inline-perms",
            "permissions": {}
        }
    }"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    // permissions field has been removed; verify id is still parsed correctly
    assert_eq!(config.id, "no-inline-perms");
    // Verify skills retains default value — proves permissions key was fully ignored.
    assert_eq!(config.skills, vec!["*"]);
}

#[test]
fn test_resolved_config_no_permissions_field() {
    // Verify that ResolvedAgentConfig can be constructed without a permissions field.
    let config = AgentConfig {
        id: "test-agent".to_string(),
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>").unwrap();
    assert_eq!(resolved.id, "test-agent");

    // Verify merge path also works without a permissions field (no panic).
    let project = AgentConfig {
        id: "test-agent".to_string(),
        model: Some("gpt-4o".to_string()),
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        ..Default::default()
    };
    let merged = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(merged.id, "test-agent");
    assert_eq!(merged.source, ConfigSource::Merged);

    // Verify default field values on resolved config.
    assert_eq!(merged.skills, vec!["*"]); // default from AgentConfig::default()
    assert_eq!(merged.tools, vec!["*"]); // default from AgentConfig::default()
    assert!(merged.disallowed_tools.is_empty());
}

#[test]
fn test_merge_tools_empty_project_falls_back_to_user() {
    // Project-level tools is empty vec → fall back to user-level tools.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        tools: vec![],
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        tools: vec!["read".to_string(), "grep".to_string()],
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(
        resolved.tools,
        vec!["read", "grep"],
        "empty project tools should fall back to user tools"
    );
}

#[test]
fn test_merge_allow_agents_empty_project_falls_back_to_user() {
    // Project-level allow_agents is empty vec → fall back to user-level.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            allow_agents: vec![],
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            allow_agents: vec!["agent-a".to_string()],
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(
        resolved.subagents.allow_agents,
        vec!["agent-a"],
        "empty project allow_agents should fall back to user allow_agents"
    );
}
