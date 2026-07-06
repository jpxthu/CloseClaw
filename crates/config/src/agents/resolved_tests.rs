//! Tests for `ResolvedAgentConfig::merge` — field-level override semantics.
//!
//! Covers Gap 3 (requireAgentId) and Gap 4 (bootstrapMode, maxSpawnDepth,
//! maxChildren) from the design doc alignment plan.

use crate::agents::config_types::{AgentConfig, SubagentsConfig};
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
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
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
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
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
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert_eq!(resolved.bootstrap_mode, BootstrapMode::Minimal);
    assert_eq!(resolved.subagents.require_agent_id, Some(true));
    assert_eq!(resolved.subagents.max_spawn_depth, Some(3));
    assert_eq!(resolved.subagents.max_children, Some(10));
}

// ------------------------------------------------------------------
// MemoryConfig field-level merge: merge_overrides
// ------------------------------------------------------------------

use crate::agents::config_types::{
    DreamingCapacityConfig, DreamingConfig, DreamingScoringConfig, DreamingThresholdConfig,
    MemoryConfig, MemoryStorageConfig, MiningConfig, SearchConfig,
};

/// Build a global MemoryConfig with non-default values.
fn make_global_memory() -> MemoryConfig {
    MemoryConfig {
        storage: MemoryStorageConfig {
            db_path: Some("global/memory.db".into()),
            memory_md_path: Some("global/MEMORY.md".into()),
        },
        mining: MiningConfig {
            enabled: Some(true),
            model: Some("global-miner".into()),
            max_events_per_session: Some(15),
            dedup_window_days: Some(60),
            ..Default::default()
        },
        dreaming: DreamingConfig {
            enabled: Some(true),
            model: Some("global-dreamer".into()),
            schedule: Some("0 2 * * *".into()),
            scoring: DreamingScoringConfig {
                frequency_weight: Some(1.0),
                recency_weight: Some(0.5),
                explicitness_weight: Some(1.5),
                cross_agent_weight: Some(1.3),
                negative_signal_weight: Some(-0.5),
            },
            threshold: DreamingThresholdConfig {
                absolute: Some(2.0),
                relative: Some(0.3),
            },
            capacity: DreamingCapacityConfig {
                max_rules: Some(20),
            },
            ..Default::default()
        },
        search: SearchConfig {
            enabled: Some(true),
            model: Some("global-search".into()),
            timeout_ms: Some(5000),
            max_summary_chars: Some(800),
            min_entity_hits: Some(2),
            top_k_events: Some(5),
            context_turns: Some(8),
        },
    }
}

// --- Per-agent no memory declaration → inherit global ---

#[test]
fn test_merge_memory_no_agent_override_inherits_global() {
    let global = make_global_memory();
    let agent = MemoryConfig::default(); // all None/default
    let merged = global.merge_overrides(&agent);

    // Mining inherits global
    assert_eq!(merged.mining.enabled, Some(true));
    assert_eq!(merged.mining.model.as_deref(), Some("global-miner"));
    assert_eq!(merged.mining.max_events_per_session, Some(15));
    assert_eq!(merged.mining.dedup_window_days, Some(60));

    // Dreaming inherits global
    assert_eq!(merged.dreaming.enabled, Some(true));
    assert_eq!(merged.dreaming.model.as_deref(), Some("global-dreamer"));
    assert_eq!(merged.dreaming.schedule.as_deref(), Some("0 2 * * *"));
    assert_eq!(merged.dreaming.threshold.absolute, Some(2.0));

    // Search inherits global
    assert_eq!(merged.search.enabled, Some(true));
    assert_eq!(merged.search.model.as_deref(), Some("global-search"));
    assert_eq!(merged.search.timeout_ms, Some(5000));
    assert_eq!(merged.search.max_summary_chars, Some(800));
    assert_eq!(merged.search.min_entity_hits, Some(2));
    assert_eq!(merged.search.top_k_events, Some(5));
    assert_eq!(merged.search.context_turns, Some(8));

    // Storage inherits global
    assert_eq!(merged.storage.db_path.as_deref(), Some("global/memory.db"));
    assert_eq!(
        merged.storage.memory_md_path.as_deref(),
        Some("global/MEMORY.md")
    );
}

// --- search.enabled override: agent false overrides global true ---

#[test]
fn test_merge_memory_search_enabled_override() {
    let global = make_global_memory();
    let agent = MemoryConfig {
        search: SearchConfig {
            enabled: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    let merged = global.merge_overrides(&agent);
    assert_eq!(merged.search.enabled, Some(false));
}

// --- dreaming.threshold.absolute override: agent 3.0 overrides global 2.0 ---

#[test]
fn test_merge_memory_dreaming_threshold_override() {
    let global = make_global_memory();
    let agent = MemoryConfig {
        dreaming: DreamingConfig {
            threshold: DreamingThresholdConfig {
                absolute: Some(3.0),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let merged = global.merge_overrides(&agent);
    assert_eq!(merged.dreaming.threshold.absolute, Some(3.0));
    // Other dreaming fields inherit global
    assert_eq!(merged.dreaming.threshold.relative, Some(0.3));
    assert_eq!(merged.dreaming.enabled, Some(true));
    assert_eq!(merged.dreaming.model.as_deref(), Some("global-dreamer"));
    assert_eq!(merged.dreaming.schedule.as_deref(), Some("0 2 * * *"));
}

// --- Per-agent full declaration → all per-agent values used ---

#[test]
fn test_merge_memory_full_agent_override() {
    let global = make_global_memory();
    let agent = MemoryConfig {
        storage: MemoryStorageConfig {
            db_path: Some("agent/db.sqlite".into()),
            memory_md_path: Some("agent/NOTES.md".into()),
        },
        mining: MiningConfig {
            enabled: Some(false),
            model: Some("agent-miner".into()),
            max_events_per_session: Some(5),
            dedup_window_days: Some(7),
            ..Default::default()
        },
        dreaming: DreamingConfig {
            enabled: Some(false),
            model: Some("agent-dreamer".into()),
            schedule: Some("0 6 * * *".into()),
            threshold: DreamingThresholdConfig {
                absolute: Some(5.0),
                relative: Some(0.8),
            },
            ..Default::default()
        },
        search: SearchConfig {
            enabled: Some(false),
            model: Some("agent-search".into()),
            timeout_ms: Some(1000),
            max_summary_chars: Some(200),
            min_entity_hits: Some(4),
            top_k_events: Some(10),
            context_turns: Some(2),
        },
    };
    let merged = global.merge_overrides(&agent);

    // All agent values used (non-default values override global)
    assert_eq!(merged.mining.enabled, Some(false));
    assert_eq!(merged.mining.model.as_deref(), Some("agent-miner"));
    assert_eq!(merged.mining.max_events_per_session, Some(5));
    assert_eq!(merged.mining.dedup_window_days, Some(7));

    assert_eq!(merged.dreaming.enabled, Some(false));
    assert_eq!(merged.dreaming.model.as_deref(), Some("agent-dreamer"));
    assert_eq!(merged.dreaming.schedule.as_deref(), Some("0 6 * * *"));
    assert_eq!(merged.dreaming.threshold.absolute, Some(5.0));
    assert_eq!(merged.dreaming.threshold.relative, Some(0.8));

    assert_eq!(merged.search.enabled, Some(false));
    assert_eq!(merged.search.model.as_deref(), Some("agent-search"));
    assert_eq!(merged.search.timeout_ms, Some(1000));
    assert_eq!(merged.search.max_summary_chars, Some(200));
    assert_eq!(merged.search.min_entity_hits, Some(4));
    assert_eq!(merged.search.top_k_events, Some(10));
    assert_eq!(merged.search.context_turns, Some(2));

    assert_eq!(merged.storage.db_path.as_deref(), Some("agent/db.sqlite"));
    assert_eq!(
        merged.storage.memory_md_path.as_deref(),
        Some("agent/NOTES.md")
    );
}

// --- Partial agent override: some fields override, rest inherit ---

#[test]
fn test_merge_memory_partial_override() {
    let global = make_global_memory();
    let agent = MemoryConfig {
        mining: MiningConfig {
            max_events_per_session: Some(5),
            ..Default::default()
        },
        dreaming: DreamingConfig {
            threshold: DreamingThresholdConfig {
                absolute: Some(3.0),
                ..Default::default()
            },
            ..Default::default()
        },
        search: SearchConfig {
            timeout_ms: Some(1000),
            ..Default::default()
        },
        ..Default::default()
    };
    let merged = global.merge_overrides(&agent);

    // Mining: enabled inherits global (Some(true)), max_events overrides
    assert_eq!(merged.mining.enabled, Some(true));
    assert_eq!(merged.mining.model.as_deref(), Some("global-miner"));
    assert_eq!(merged.mining.max_events_per_session, Some(5));
    assert_eq!(merged.mining.dedup_window_days, Some(60));

    // Dreaming: enabled inherits, threshold.absolute overrides
    assert_eq!(merged.dreaming.enabled, Some(true));
    assert_eq!(merged.dreaming.threshold.absolute, Some(3.0));
    assert_eq!(merged.dreaming.threshold.relative, Some(0.3));
    assert_eq!(merged.dreaming.model.as_deref(), Some("global-dreamer"));

    // Search: enabled inherits, timeout overrides
    assert_eq!(merged.search.enabled, Some(true));
    assert_eq!(merged.search.timeout_ms, Some(1000));
    assert_eq!(merged.search.model.as_deref(), Some("global-search"));
    assert_eq!(merged.search.max_summary_chars, Some(800));
}

// --- Merge with both global and agent having enabled=None ---

#[test]
fn test_merge_memory_enabled_none_both_levels() {
    let global = MemoryConfig::default(); // enabled=None
    let agent = MemoryConfig::default(); // enabled=None
    let merged = global.merge_overrides(&agent);
    assert_eq!(merged.mining.enabled, None);
    assert_eq!(merged.dreaming.enabled, None);
    assert_eq!(merged.search.enabled, None);
}

// --- Merge global enabled=false, agent enabled=true ---

#[test]
fn test_merge_memory_agent_enables_over_global_disabled() {
    let global = MemoryConfig {
        mining: MiningConfig {
            enabled: Some(false),
            ..Default::default()
        },
        search: SearchConfig {
            enabled: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    let agent = MemoryConfig {
        mining: MiningConfig {
            enabled: Some(true),
            ..Default::default()
        },
        search: SearchConfig {
            enabled: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let merged = global.merge_overrides(&agent);
    assert_eq!(merged.mining.enabled, Some(true));
    assert_eq!(merged.search.enabled, Some(true));
}

// --- Dreaming sub-config merge: scoring weights ---

#[test]
fn test_merge_memory_dreaming_scoring_override() {
    let global = make_global_memory();
    let agent = MemoryConfig {
        dreaming: DreamingConfig {
            scoring: DreamingScoringConfig {
                frequency_weight: Some(3.0),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let merged = global.merge_overrides(&agent);
    // frequency_weight overridden
    assert_eq!(merged.dreaming.scoring.frequency_weight, Some(3.0));
    // Other scoring weights inherit global defaults (not overridden by agent)
    assert_eq!(merged.dreaming.scoring.recency_weight, Some(0.5));
    assert_eq!(merged.dreaming.scoring.explicitness_weight, Some(1.5));
}

// --- from_single with memory ---

#[test]
fn test_from_single_with_memory_config() {
    let config = AgentConfig {
        id: "test-agent".to_string(),
        memory: Some(MemoryConfig {
            search: SearchConfig {
                enabled: Some(true),
                timeout_ms: Some(10000),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert_eq!(resolved.memory.search.enabled, Some(true));
    assert_eq!(resolved.memory.search.timeout_ms, Some(10000));
}

#[test]
fn test_from_single_without_memory_uses_default() {
    let config = AgentConfig {
        id: "test-agent".to_string(),
        memory: None,
        ..Default::default()
    };
    let resolved =
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "<test>", None).unwrap();
    assert_eq!(resolved.memory, MemoryConfig::default());
}

// --- Merge project+user with different memory configs ---

#[test]
fn test_merge_project_user_memory_field_level() {
    let user = AgentConfig {
        id: "test-agent".to_string(),
        memory: Some(MemoryConfig {
            mining: MiningConfig {
                enabled: Some(true),
                max_events_per_session: Some(20),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let project = AgentConfig {
        id: "test-agent".to_string(),
        memory: Some(MemoryConfig {
            mining: MiningConfig {
                dedup_window_days: Some(7),
                ..Default::default()
            },
            search: SearchConfig {
                enabled: Some(true),
                timeout_ms: Some(8000),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();

    // Project overrides user where specified
    assert_eq!(resolved.memory.mining.dedup_window_days, Some(7));
    assert_eq!(resolved.memory.search.enabled, Some(true));
    assert_eq!(resolved.memory.search.timeout_ms, Some(8000));

    // User's non-overridden fields preserved
    assert_eq!(resolved.memory.mining.enabled, Some(true));
    assert_eq!(resolved.memory.mining.max_events_per_session, Some(20));
}

// --- Edge: project overrides user's enabled=false ---

#[test]
fn test_merge_project_user_enabled_override() {
    let user = AgentConfig {
        id: "test-agent".to_string(),
        memory: Some(MemoryConfig {
            search: SearchConfig {
                enabled: Some(true),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let project = AgentConfig {
        id: "test-agent".to_string(),
        memory: Some(MemoryConfig {
            search: SearchConfig {
                enabled: Some(false),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let resolved = ResolvedAgentConfig::merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.memory.search.enabled, Some(false));
}
