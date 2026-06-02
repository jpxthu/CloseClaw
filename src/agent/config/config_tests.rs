use super::*;

use crate::config::agents::{ConfigSource, ResolvedAgentConfig};

use tempfile::TempDir;

#[test]
fn test_agent_config_save_load() {
    let temp = TempDir::new().unwrap();
    let config = AgentConfig {
        id: "test-id".to_string(),
        name: Some("Test Agent".to_string()),
        parent_id: Some("parent-id".to_string()),
        max_child_depth: 2,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: CommunicationConfig {
            outbound: vec!["parent-id".to_string()],
            inbound: vec!["parent-id".to_string()],
        },
        ..Default::default()
    };

    let path = temp.path().join("config.json");
    config.save(&path).unwrap();
    let loaded = AgentConfig::load(&path).unwrap();

    assert_eq!(loaded.id, config.id);
    assert_eq!(loaded.name, config.name);
    assert_eq!(loaded.parent_id, config.parent_id);
    assert_eq!(loaded.max_child_depth, config.max_child_depth);
    assert_eq!(loaded.communication.outbound, config.communication.outbound);
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
    let with_parent = CommunicationConfig::default_with_parent(Some("parent-1"));
    assert_eq!(with_parent.outbound, vec!["parent-1"]);
    assert_eq!(with_parent.inbound, vec!["parent-1"]);

    let without_parent = CommunicationConfig::default_with_parent(None);
    assert!(without_parent.outbound.is_empty());
    assert!(without_parent.inbound.is_empty());
}

#[test]
fn test_communication_allowed() {
    let parent = AgentConfig {
        id: "parent-1".to_string(),
        name: Some("Parent".to_string()),
        parent_id: None,
        max_child_depth: 2,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: CommunicationConfig {
            outbound: vec!["child-1".to_string()],
            inbound: vec!["child-1".to_string()],
        },
        ..Default::default()
    };

    let child = AgentConfig {
        id: "child-1".to_string(),
        name: Some("Child".to_string()),
        parent_id: Some("parent-1".to_string()),
        max_child_depth: 1,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: CommunicationConfig::default_with_parent(Some("parent-1")),
        ..Default::default()
    };

    // Parent -> Child should be allowed
    let result = check_communication_allowed(&parent, &child);
    assert_eq!(result, CommunicationCheckResult::Allowed);

    // Child -> Parent should be allowed
    let result = check_communication_allowed(&child, &parent);
    assert_eq!(result, CommunicationCheckResult::Allowed);
}

#[test]
fn test_communication_denied_outbound() {
    let agent_a = AgentConfig {
        id: "agent-a".to_string(),
        name: Some("Agent A".to_string()),
        parent_id: None,
        max_child_depth: 2,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: CommunicationConfig {
            outbound: vec!["agent-b".to_string()],
            inbound: vec!["agent-b".to_string()],
        },
        ..Default::default()
    };

    let agent_c = AgentConfig {
        id: "agent-c".to_string(),
        name: Some("Agent C".to_string()),
        parent_id: None,
        max_child_depth: 2,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: CommunicationConfig {
            outbound: vec![],
            inbound: vec![],
        },
        ..Default::default()
    };

    // Agent A -> Agent C: A's outbound doesn't contain C
    let result = check_communication_allowed(&agent_a, &agent_c);
    assert_eq!(result, CommunicationCheckResult::TargetNotInSourceOutbound);
}

#[test]
fn test_communication_denied_inbound() {
    let agent_a = AgentConfig {
        id: "agent-a".to_string(),
        name: Some("Agent A".to_string()),
        parent_id: None,
        max_child_depth: 2,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: CommunicationConfig {
            outbound: vec!["agent-b".to_string()],
            inbound: vec!["agent-b".to_string()],
        },
        ..Default::default()
    };

    let agent_b = AgentConfig {
        id: "agent-b".to_string(),
        name: Some("Agent B".to_string()),
        parent_id: None,
        max_child_depth: 2,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: CommunicationConfig {
            outbound: vec![],
            inbound: vec![], // B doesn't accept inbound from anyone
        },
        ..Default::default()
    };

    // Agent A -> Agent B: A's outbound contains B, but B's inbound doesn't contain A
    let result = check_communication_allowed(&agent_a, &agent_b);
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
    let config = AgentConfig {
        id: "test-agent".to_string(),
        name: Some("Test".to_string()),
        model: Some("gpt-4o".to_string()),
        workspace: Some("/tmp/workspace".to_string()),
        agent_dir: Some("/tmp/agent_dir".to_string()),
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
    assert_eq!(deserialized.workspace, Some("/tmp/workspace".to_string()));
    assert_eq!(deserialized.agent_dir, Some("/tmp/agent_dir".to_string()));
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

#[test]
fn test_max_depth_allowed() {
    // Root agent with max_child_depth=3, currently at depth 0
    let root = AgentConfig {
        id: "root".to_string(),
        name: Some("Root".to_string()),
        parent_id: None,
        max_child_depth: 3,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: Default::default(),
        ..Default::default()
    };

    // No parents, so depth = 0, max_child_depth = 3
    // Can spawn (0 < 3), max_allowed for child = 2
    let result = check_max_depth(&root, |_: &str| None);
    match result {
        MaxDepthCheckResult::Allowed {
            current_depth,
            max_allowed,
        } => {
            assert_eq!(current_depth, 0);
            assert_eq!(max_allowed, 2); // 3 - 1
        }
        _ => panic!("expected Allowed"),
    }
}

#[test]
fn test_max_depth_exceeded() {
    // Agent at depth 3, max_child_depth=2 (already exceeded!)
    let leaf = AgentConfig {
        id: "leaf".to_string(),
        name: Some("Leaf".to_string()),
        parent_id: Some("parent".to_string()),
        max_child_depth: 2,
        created_at: Utc::now(),
        state: AgentConfigState::Running,
        communication: Default::default(),
        ..Default::default()
    };

    // Simulate: root -> child1 -> child2 -> leaf (depth 3)
    let get_parent = |id: &str| match id {
        "parent" => Some(AgentConfig {
            id: "parent".to_string(),
            name: Some("Parent".to_string()),
            parent_id: Some("grandparent".to_string()),
            max_child_depth: 2,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: Default::default(),
            ..Default::default()
        }),
        "grandparent" => Some(AgentConfig {
            id: "grandparent".to_string(),
            name: Some("Grandparent".to_string()),
            parent_id: None,
            max_child_depth: 3,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: Default::default(),
            ..Default::default()
        }),
        _ => None,
    };

    let result = check_max_depth(&leaf, get_parent);
    match result {
        MaxDepthCheckResult::ExceedsMaxDepth {
            current_depth,
            max_child_depth,
        } => {
            // leaf has 2 ancestors (parent + grandparent) = depth 2
            assert_eq!(current_depth, 2);
            assert_eq!(max_child_depth, 2);
        }
        _ => panic!("expected ExceedsMaxDepth"),
    }
}

// =====================================================================
// Step 1.5 — Tests for `AgentConfig.name` becoming optional and
// `ResolvedAgentConfig` falling back to `id` when name is missing/empty.
// =====================================================================

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
