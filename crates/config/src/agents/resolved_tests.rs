//! Tests for `ResolvedAgentConfig::merge` — field-level override semantics.
//!
//! Covers Gap 3 (requireAgentId) and Gap 4 (bootstrapMode, maxSpawnDepth,
//! maxChildren) from the design doc alignment plan.

use crate::agents::config_types::{AgentConfig, SubagentsConfig};
use closeclaw_common::{BootstrapMode, HookConfig, HookParams, HookType};

use super::{ConfigSource, ResolvedAgentConfig};

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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();

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
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
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
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
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
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
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
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
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
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
    assert!(resolved.hooks.is_empty());
}
