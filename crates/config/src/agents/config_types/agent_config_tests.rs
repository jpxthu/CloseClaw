//! Serde / default tests for the raw `AgentConfig` and permissions types.
//!
//! Migrated from `closeclaw_agent::config::config_tests` when the agent
//! crate dropped its `closeclaw-config` dependency (issue #3344): these
//! types are defined here, so their behavioral tests live here too.

#![allow(deprecated)] // default_child_agent is deprecated; tests cover its parsing

use std::collections::HashMap;

use super::*;
use closeclaw_common::{BootstrapMode, ModelSpec, SubagentsConfig};
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
    let content = serde_json::to_string_pretty(&config).unwrap();
    std::fs::write(&path, content).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    let loaded: AgentConfig = serde_json::from_str(&raw).unwrap();

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
        "command".to_string(),
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
    let content = serde_json::to_string_pretty(&permissions).unwrap();
    std::fs::write(&path, content).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    let loaded: AgentPermissions = serde_json::from_str(&raw).unwrap();

    assert_eq!(loaded.agent_id, permissions.agent_id);
    assert!(loaded.is_allowed("command"));
    assert!(!loaded.is_allowed("network"));
}

#[test]
fn test_agent_config_new_fields_defaults() {
    let config = AgentConfig::default();

    // New fields should have sensible defaults
    assert!(config.model.is_none());
    assert!(config.workspace.is_none());
    assert!(config.agent_dir.is_none());
    assert_eq!(config.bootstrap_mode, None);
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
        model: Some(ModelSpec::single("gpt-4o")),
        workspace: Some(workspace_path.to_str().unwrap().to_string()),
        agent_dir: Some(agent_dir_path.to_str().unwrap().to_string()),
        bootstrap_mode: Some(BootstrapMode::Minimal),
        skills: vec!["skill-a".to_string(), "skill-b".to_string()],
        tools: vec!["read".to_string(), "write".to_string()],
        disallowed_tools: vec!["exec".to_string()],
        subagents: SubagentsConfig {
            allow_agents: vec!["agent-1".to_string()],
            require_agent_id: Some(true),
            max_spawn_depth: Some(3),
            max_children: Some(10),
            timeout: None,
            timeout_warning: None,
            timeout_notify_interval_ratio: None,
            default_child_agent: Some("child-agent".to_string()),
            model: Some(ModelSpec::single("claude-3")),
        },
        ..Default::default()
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: AgentConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.model, Some(ModelSpec::single("gpt-4o")));
    assert_eq!(
        deserialized.workspace,
        Some(workspace_path.to_str().unwrap().to_string())
    );
    assert_eq!(
        deserialized.agent_dir,
        Some(agent_dir_path.to_str().unwrap().to_string())
    );
    assert_eq!(deserialized.bootstrap_mode, Some(BootstrapMode::Minimal));
    assert_eq!(deserialized.skills, vec!["skill-a", "skill-b"]);
    assert_eq!(deserialized.tools, vec!["read", "write"]);
    assert_eq!(deserialized.disallowed_tools, vec!["exec"]);
    assert_eq!(deserialized.subagents.allow_agents, vec!["agent-1"]);
    assert_eq!(deserialized.subagents.require_agent_id, Some(true));
    assert_eq!(deserialized.subagents.max_spawn_depth, Some(3));
    assert_eq!(deserialized.subagents.max_children, Some(10));
    assert_eq!(
        deserialized.subagents.default_child_agent,
        Some("child-agent".to_string())
    );
    assert_eq!(
        deserialized.subagents.model,
        Some(ModelSpec::single("claude-3"))
    );
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
    assert_eq!(config.model, Some(ModelSpec::single("deepseek-v3")));
    assert_eq!(config.workspace, Some("/home/user/project".to_string()));
    assert_eq!(
        config.agent_dir,
        Some("/home/user/.agents/my-agent".to_string())
    );
    assert_eq!(config.bootstrap_mode, Some(BootstrapMode::Minimal));
    assert_eq!(config.skills, vec!["coding", "search"]);
    assert_eq!(config.tools, vec!["read", "write", "exec"]);
    assert_eq!(config.disallowed_tools, vec!["web_search"]);
    assert_eq!(
        config.subagents.allow_agents,
        vec!["coding-agent", "review-agent"]
    );
    assert_eq!(config.subagents.require_agent_id, Some(true));
    assert_eq!(config.subagents.max_spawn_depth, Some(2));
    assert_eq!(config.subagents.max_children, Some(8));
    assert_eq!(
        config.subagents.default_child_agent,
        Some("coding-agent".to_string())
    );
    assert_eq!(
        config.subagents.model,
        Some(ModelSpec::single("gpt-4o-mini"))
    );
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
    assert_eq!(
        config.model,
        Some(ModelSpec::single("deepseek/deepseek-chat"))
    );
    assert_eq!(config.workspace, None);
    assert_eq!(config.agent_dir, None);
    assert_eq!(config.bootstrap_mode, Some(BootstrapMode::Minimal));
    assert_eq!(config.skills, vec!["code-review"]);
    assert_eq!(
        config.tools,
        vec!["read", "grep", "glob", "web_search", "web_fetch"]
    );
    assert!(config.disallowed_tools.is_empty());
    assert!(config.subagents.allow_agents.is_empty());
    assert_eq!(config.subagents.require_agent_id, None);
    assert_eq!(config.subagents.max_spawn_depth, Some(0));
    assert_eq!(config.subagents.max_children, Some(0));
    assert_eq!(config.subagents.default_child_agent, None);
    assert_eq!(config.subagents.model, None);
}

// Tests for optional name and id fallback (serde layer)

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
    // layer. The empty-string → id fallback happens later in the config
    // crate's `from_single` / `merge` construction functions, not in serde.
    let json = r#"{"id": "x", "name": ""}"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.id, "x");
    assert_eq!(config.name, Some("".to_string()));
}

// Reverse serde: unknown fields from removed fields are silently ignored.
// This ensures backward compatibility with old config files that still
// contain deprecated fields.

#[test]
fn test_agent_config_ignores_removed_fields() {
    // JSON contains fields removed by earlier design-doc alignment rounds:
    // - max_child_depth
    // - state, created_at
    // - communication, wait_timeout_secs, grace_period_secs
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
    assert_eq!(config.model, Some(ModelSpec::single("gpt-4o")));

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

// =====================================================================
// ModelSpec parsing through AgentConfig
// =====================================================================

#[test]
fn test_model_spec_string_format_parses_to_single() {
    let json = r#"{"id": "a", "model": "gpt-4o"}"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.model, Some(ModelSpec::single("gpt-4o")));
}

#[test]
fn test_model_spec_object_format_with_fallback() {
    let json = r#"{
        "id": "a",
        "model": {"primary": "gpt-4o", "fallback": ["claude-3", "gemini"]}
    }"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.model,
        Some(ModelSpec::with_fallback(
            "gpt-4o",
            vec!["claude-3".to_string(), "gemini".to_string()]
        ))
    );
}

#[test]
fn test_model_spec_object_format_empty_fallback() {
    let json = r#"{
        "id": "a",
        "model": {"primary": "gpt-4o", "fallback": []}
    }"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.model, Some(ModelSpec::single("gpt-4o")));
}

#[test]
fn test_model_spec_object_format_missing_fallback() {
    let json = r#"{
        "id": "a",
        "model": {"primary": "gpt-4o"}
    }"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.model, Some(ModelSpec::single("gpt-4o")));
}

#[test]
fn test_model_spec_object_format_missing_primary_is_error() {
    let json = r#"{
        "id": "a",
        "model": {"fallback": ["claude-3"]}
    }"#;
    let result = serde_json::from_str::<AgentConfig>(json);
    assert!(result.is_err(), "missing primary field should cause error");
}

#[test]
fn test_model_spec_subagents_string_format() {
    let json = r#"{
        "id": "a",
        "subagents": {"model": "claude-3"}
    }"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.subagents.model, Some(ModelSpec::single("claude-3")));
}

#[test]
fn test_model_spec_subagents_object_format() {
    let json = r#"{
        "id": "a",
        "subagents": {
            "model": {"primary": "gpt-4o", "fallback": ["deepseek"]}
        }
    }"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.subagents.model,
        Some(ModelSpec::with_fallback(
            "gpt-4o",
            vec!["deepseek".to_string()]
        ))
    );
}

#[test]
fn test_model_spec_roundtrip_string() {
    let config = AgentConfig {
        id: "roundtrip".to_string(),
        model: Some(ModelSpec::single("gpt-4o")),
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    let parsed: AgentConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.model, Some(ModelSpec::single("gpt-4o")));
}

#[test]
fn test_model_spec_roundtrip_with_fallback() {
    let config = AgentConfig {
        id: "roundtrip".to_string(),
        model: Some(ModelSpec::with_fallback(
            "gpt-4o",
            vec!["claude-3".to_string()],
        )),
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    let parsed: AgentConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(
        parsed.model,
        Some(ModelSpec::with_fallback(
            "gpt-4o",
            vec!["claude-3".to_string()]
        ))
    );
}
