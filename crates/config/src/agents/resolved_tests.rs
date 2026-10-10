//! Tests for `merge` — field-level override semantics.
//!
//! Covers Gap 3 (requireAgentId) and Gap 4 (bootstrapMode, maxSpawnDepth,
//! maxChildren) from the design doc alignment plan.

use crate::agents::config_types::AgentConfig;
use closeclaw_common::agent_config::{ConfigSource, SubagentsConfig};
use closeclaw_common::{BootstrapMode, HookConfig, HookParams, HookType, ModelSpec};

use super::{from_single, merge};

// ------------------------------------------------------------------
// Helper
// ------------------------------------------------------------------

fn make_user_config() -> AgentConfig {
    AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: Some(BootstrapMode::Minimal),
        subagents: SubagentsConfig {
            require_agent_id: Some(true),
            max_spawn_depth: Some(3),
            max_children: Some(10),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ------------------------------------------------------------------
// Gap 4: bootstrapMode field-level override
// ------------------------------------------------------------------

#[test]
fn test_merge_project_bootstrap_mode_overrides_user() {
    // Project explicitly sets "full" → should override user's "minimal".
    let project = AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: Some(BootstrapMode::Full),
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Full);
}

#[test]
fn test_merge_project_bootstrap_mode_minimal_overrides_user_full() {
    // Boundary: project explicitly sets "minimal" → overrides user's "full".
    let user = AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: Some(BootstrapMode::Full),
        ..Default::default()
    };
    let project = AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: Some(BootstrapMode::Minimal),
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Minimal);
}

#[test]
fn test_merge_project_bootstrap_mode_none_falls_back_to_user() {
    // Project not specified (None) → fall back to user's "minimal".
    let project = AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: None,
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Minimal);
}

#[test]
fn test_merge_both_bootstrap_mode_none_uses_default() {
    // Both levels unspecified → default is BootstrapMode::Full.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: None,
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: None,
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Full);
}

// ------------------------------------------------------------------
// Gap 3: requireAgentId field-level override
// ------------------------------------------------------------------

#[test]
fn test_merge_project_require_agent_id_false_overrides_user_true() {
    // Project explicitly sets false → should override user's true.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            require_agent_id: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.require_agent_id, Some(false));
}

#[test]
fn test_merge_project_require_agent_id_true_overrides_user_false() {
    // Boundary: project explicitly sets true → overrides user's false.
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            require_agent_id: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            require_agent_id: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.require_agent_id, Some(true));
}

#[test]
fn test_merge_project_require_agent_id_none_falls_back_to_user() {
    // Project not specified (None) → fall back to user's true.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            require_agent_id: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.require_agent_id, Some(true));
}

#[test]
fn test_merge_both_require_agent_id_none_uses_default() {
    // Both levels unspecified → filled with default (false).
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            require_agent_id: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            require_agent_id: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.require_agent_id, Some(false));
}

// ------------------------------------------------------------------
// Gap 4: maxSpawnDepth field-level override
// ------------------------------------------------------------------

#[test]
fn test_merge_project_max_spawn_depth_overrides_user() {
    // Project explicitly sets 1 → should override user's 3.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_spawn_depth: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.max_spawn_depth, Some(1));
}

#[test]
fn test_merge_project_max_spawn_depth_zero_overrides_user() {
    // Boundary: project explicitly sets 0 → overrides user's 3.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_spawn_depth: Some(0),
            ..Default::default()
        },
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.max_spawn_depth, Some(0));
}

#[test]
fn test_merge_project_max_spawn_depth_none_falls_back_to_user() {
    // Project not specified (None) → fall back to user's 3.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_spawn_depth: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.max_spawn_depth, Some(3));
}

#[test]
fn test_merge_both_max_spawn_depth_none_uses_default() {
    // Both levels unspecified → default is 1.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_spawn_depth: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_spawn_depth: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.max_spawn_depth, Some(1));
}

// ------------------------------------------------------------------
// Gap 4: maxChildren field-level override
// ------------------------------------------------------------------

#[test]
fn test_merge_project_max_children_overrides_user() {
    // Project explicitly sets 5 → should override user's 10.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_children: Some(5),
            ..Default::default()
        },
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.max_children, Some(5));
}

#[test]
fn test_merge_project_max_children_zero_overrides_user() {
    // Boundary: project explicitly sets 0 → overrides user's 10.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_children: Some(0),
            ..Default::default()
        },
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.max_children, Some(0));
}

#[test]
fn test_merge_project_max_children_none_falls_back_to_user() {
    // Project not specified (None) → fall back to user's 10.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_children: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.max_children, Some(10));
}

#[test]
fn test_merge_both_max_children_none_uses_default() {
    // Both levels unspecified → default is 5.
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_children: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_children: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.max_children, Some(5));
}

// ------------------------------------------------------------------
// JSON backward compatibility: old format without optional fields
// ------------------------------------------------------------------

#[test]
fn test_merge_old_json_without_optional_fields() {
    // Old JSON without bootstrapMode, requireAgentId, maxSpawnDepth,
    // maxChildren should deserialize as None and fall back to user/defaults.
    let json = r#"{"id":"old-agent"}"#;
    let project: AgentConfig = serde_json::from_str(json).unwrap();
    let user = make_user_config();
    let resolved = merge(project, user, "<test>", None).unwrap();

    // All should fall back to user values
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(resolved.subagents.require_agent_id, Some(true));
    assert_eq!(resolved.subagents.max_spawn_depth, Some(3));
    assert_eq!(resolved.subagents.max_children, Some(10));
}

// ------------------------------------------------------------------
// from_single: Option<T> → resolved with defaults
// ------------------------------------------------------------------

#[test]
fn test_from_single_resolves_bootstrap_mode_default() {
    let config = AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: None,
        ..Default::default()
    };
    let resolved = from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Full);
}

#[test]
fn test_from_single_fills_subagent_defaults() {
    // from_single now fills subagent defaults via apply_subagent_defaults.
    let config = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_spawn_depth: None,
            max_children: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.require_agent_id, Some(false));
    assert_eq!(resolved.subagents.max_spawn_depth, Some(1));
    assert_eq!(resolved.subagents.max_children, Some(5));
}

#[test]
fn test_from_single_preserves_explicit_values() {
    let config = AgentConfig {
        id: "test-agent".to_string(),
        bootstrap_mode: Some(BootstrapMode::Minimal),
        subagents: SubagentsConfig {
            require_agent_id: Some(true),
            max_spawn_depth: Some(3),
            max_children: Some(10),
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(resolved.subagents.require_agent_id, Some(true));
    assert_eq!(resolved.subagents.max_spawn_depth, Some(3));
    assert_eq!(resolved.subagents.max_children, Some(10));
}

// ------------------------------------------------------------------
// Gap 2: hooks field wiring (Step 1.6)
// ------------------------------------------------------------------

#[test]
fn test_from_single_preserves_hooks() {
    let hooks = vec![
        HookConfig {
            hook_type: HookType::PlanCheck,
            enabled: true,
            params: HookParams::default(),
        },
        HookConfig {
            hook_type: HookType::LoopCheck,
            enabled: true,
            params: HookParams {
                loop_check_repetition_threshold: 5,
                ..Default::default()
            },
        },
    ];
    let config = AgentConfig {
        id: "test-agent".to_string(),
        hooks: hooks.clone(),
        ..Default::default()
    };
    let resolved = from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert_eq!(resolved.hooks.len(), 2);
    assert_eq!(resolved.hooks[0].hook_type, HookType::PlanCheck);
    assert_eq!(resolved.hooks[0].enabled, true);
    assert_eq!(resolved.hooks[1].hook_type, HookType::LoopCheck);
    assert_eq!(resolved.hooks[1].params.loop_check_repetition_threshold, 5);
}

#[test]
fn test_from_single_empty_hooks_default() {
    let config = AgentConfig {
        id: "test-agent".to_string(),
        hooks: vec![],
        ..Default::default()
    };
    let resolved = from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert!(resolved.hooks.is_empty());
}

#[test]
fn test_merge_project_hooks_override_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        hooks: vec![HookConfig {
            hook_type: HookType::ProgressCheck,
            enabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        hooks: vec![HookConfig {
            hook_type: HookType::PlanCheck,
            enabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    // Project's non-empty hooks should override user's.
    assert_eq!(resolved.hooks.len(), 1);
    assert_eq!(resolved.hooks[0].hook_type, HookType::ProgressCheck);
}

#[test]
fn test_merge_project_hooks_empty_falls_back_to_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        hooks: vec![],
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        hooks: vec![HookConfig {
            hook_type: HookType::LoopCheck,
            enabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    // Empty project falls back to user.
    assert_eq!(resolved.hooks.len(), 1);
    assert_eq!(resolved.hooks[0].hook_type, HookType::LoopCheck);
}

#[test]
fn test_merge_both_hooks_empty_default() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        hooks: vec![],
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        hooks: vec![],
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert!(resolved.hooks.is_empty());
}

// ------------------------------------------------------------------
// Name fallback: None / Some("") → resolved id
// (migrated from closeclaw_agent::config::config_tests, issue #3344)
// ------------------------------------------------------------------

#[test]
fn test_resolved_config_name_fallback_to_id() {
    // `name = None` → resolved `name` must equal `id`.
    let config = AgentConfig {
        id: "agent-x".to_string(),
        name: None,
        ..Default::default()
    };
    let resolved = from_single(config, ConfigSource::User, "<test>", None).unwrap();
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
    let resolved = from_single(config, ConfigSource::User, "<test>", None).unwrap();
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
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.id, "agent-z");
    assert_eq!(resolved.name, "agent-z");
    assert_eq!(resolved.source, ConfigSource::Merged);
}

// ------------------------------------------------------------------
// Vec fields: project `["*"]` overrides user; empty project falls
// back to user (skills / tools / allow_agents)
// ------------------------------------------------------------------

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
    let resolved = merge(project, user, "<test>", None).unwrap();
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
    let resolved = merge(project, user, "<test>", None).unwrap();
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
    let resolved = merge(project, user, "<test>", None).unwrap();
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
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(
        resolved.skills,
        vec!["user-skill".to_string()],
        "empty project skills should fall back to user skills"
    );
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
    let resolved = merge(project, user, "<test>", None).unwrap();
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
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(
        resolved.subagents.allow_agents,
        vec!["agent-a"],
        "empty project allow_agents should fall back to user allow_agents"
    );
}

// ------------------------------------------------------------------
// Resolved construction works without a permissions field
// ------------------------------------------------------------------

#[test]
fn test_resolved_config_no_permissions_field() {
    // Verify that ResolvedAgentConfig can be constructed without a permissions field.
    let config = AgentConfig {
        id: "test-agent".to_string(),
        ..Default::default()
    };
    let resolved = from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert_eq!(resolved.id, "test-agent");

    // Verify merge path also works without a permissions field (no panic).
    let project = AgentConfig {
        id: "test-agent".to_string(),
        model: Some(ModelSpec::single("gpt-4o")),
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        ..Default::default()
    };
    let merged = merge(project, user, "<test>", None).unwrap();
    assert_eq!(merged.id, "test-agent");
    assert_eq!(merged.source, ConfigSource::Merged);

    // Verify default field values on resolved config.
    assert_eq!(merged.skills, vec!["*"]); // default from AgentConfig::default()
    assert_eq!(merged.tools, vec!["*"]); // default from AgentConfig::default()
    assert!(merged.disallowed_tools.is_empty());
}

// ------------------------------------------------------------------
// Model merge semantics (project overrides user / fallback to user)
// ------------------------------------------------------------------

#[test]
fn test_merge_model_project_overrides_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        model: Some(ModelSpec::single("gpt-4o")),
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        model: Some(ModelSpec::single("claude-3")),
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.model, Some(ModelSpec::single("gpt-4o")));
}

#[test]
fn test_merge_model_project_with_fallback_overrides_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        model: Some(ModelSpec::with_fallback(
            "gpt-4o",
            vec!["claude-3".to_string()],
        )),
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        model: Some(ModelSpec::single("deepseek")),
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(
        resolved.model,
        Some(ModelSpec::with_fallback(
            "gpt-4o",
            vec!["claude-3".to_string()]
        ))
    );
}

#[test]
fn test_merge_model_project_none_falls_back_to_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        model: None,
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        model: Some(ModelSpec::single("claude-3")),
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.model, Some(ModelSpec::single("claude-3")));
}

#[test]
fn test_merge_model_both_none() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        model: None,
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        model: None,
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.model, None);
}

#[test]
fn test_merge_subagents_model_project_overrides_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            model: Some(ModelSpec::single("gpt-4o")),
            timeout: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            model: Some(ModelSpec::with_fallback(
                "claude-3",
                vec!["gemini".to_string()],
            )),
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.model, Some(ModelSpec::single("gpt-4o")));
}

#[test]
fn test_merge_subagents_model_project_none_falls_back_to_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            model: None,
            timeout: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            model: Some(ModelSpec::single("claude-3")),
            timeout: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(
        resolved.subagents.model,
        Some(ModelSpec::single("claude-3"))
    );
}
