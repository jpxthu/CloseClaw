//! Tests for `ResolvedAgentConfig::merge` — field-level override semantics.
//!
//! Covers Gap 3 (requireAgentId) and Gap 4 (bootstrapMode, maxSpawnDepth,
//! maxChildren) from the design doc alignment plan.

use closeclaw_common::agent_config::{AgentConfig, SubagentsConfig};
use closeclaw_common::BootstrapMode;

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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(resolved.subagents.require_agent_id, Some(true));
}

#[test]
fn test_merge_both_require_agent_id_none_uses_none() {
    // Both levels unspecified → None (no default for require_agent_id).
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
    assert_eq!(resolved.subagents.require_agent_id, None);
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>").unwrap();

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
    let resolved = ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>").unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Full);
}

#[test]
fn test_from_single_preserves_subagent_none_values() {
    // from_single passes subagents through as-is; defaults are only
    // applied during merge. None values remain None.
    let config = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            max_spawn_depth: None,
            max_children: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>").unwrap();
    assert_eq!(resolved.subagents.max_spawn_depth, None);
    assert_eq!(resolved.subagents.max_children, None);
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
    let resolved = ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>").unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(resolved.subagents.require_agent_id, Some(true));
    assert_eq!(resolved.subagents.max_spawn_depth, Some(3));
    assert_eq!(resolved.subagents.max_children, Some(10));
}
