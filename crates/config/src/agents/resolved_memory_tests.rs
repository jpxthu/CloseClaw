//! Tests for `ResolvedAgentConfig` memory config merge.

use crate::agents::config_types::AgentConfig;

use super::{from_single, merge};

// ------------------------------------------------------------------
// MemoryConfig field-level merge: merge_overrides
// ------------------------------------------------------------------

use closeclaw_common::{MemoryConfig, SubagentsConfig};

/// Build a global MemoryConfig with non-default values.
fn make_global_memory() -> MemoryConfig {
    let mut mem = MemoryConfig::default();
    mem.storage.db_path = Some("global/memory.db".into());
    mem.storage.memory_md_path = Some("global/MEMORY.md".into());
    mem.mining.enabled = Some(true);
    mem.mining.model = Some("global-miner".into());
    mem.mining.max_events_per_session = Some(15);
    mem.mining.dedup_window_days = Some(60);
    mem.dreaming.enabled = Some(true);
    mem.dreaming.model = Some("global-dreamer".into());
    mem.dreaming.schedule = Some("0 2 * * *".into());
    mem.dreaming.scoring.frequency_weight = Some(1.0);
    mem.dreaming.scoring.recency_weight = Some(0.5);
    mem.dreaming.scoring.explicitness_weight = Some(1.5);
    mem.dreaming.scoring.cross_agent_weight = Some(1.3);
    mem.dreaming.scoring.negative_signal_weight = Some(-0.5);
    mem.dreaming.threshold.absolute = Some(2.0);
    mem.dreaming.threshold.relative = Some(0.3);
    mem.dreaming.capacity.max_rules = Some(20);
    mem.search.enabled = Some(true);
    mem.search.model = Some("global-search".into());
    mem.search.timeout_ms = Some(5000);
    mem.search.max_summary_chars = Some(800);
    mem.search.min_entity_hits = Some(2);
    mem.search.top_k_events = Some(5);
    mem.search.context_turns = Some(8);
    mem
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
    let mut agent = MemoryConfig::default();
    agent.search.enabled = Some(false);
    let merged = global.merge_overrides(&agent);
    assert_eq!(merged.search.enabled, Some(false));
}

// --- dreaming.threshold.absolute override: agent 3.0 overrides global 2.0 ---

#[test]
fn test_merge_memory_dreaming_threshold_override() {
    let global = make_global_memory();
    let mut agent = MemoryConfig::default();
    agent.dreaming.threshold.absolute = Some(3.0);
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
    let mut agent = MemoryConfig::default();
    agent.storage.db_path = Some("agent/db.sqlite".into());
    agent.storage.memory_md_path = Some("agent/NOTES.md".into());
    agent.mining.enabled = Some(false);
    agent.mining.model = Some("agent-miner".into());
    agent.mining.max_events_per_session = Some(5);
    agent.mining.dedup_window_days = Some(7);
    agent.dreaming.enabled = Some(false);
    agent.dreaming.model = Some("agent-dreamer".into());
    agent.dreaming.schedule = Some("0 6 * * *".into());
    agent.dreaming.threshold.absolute = Some(5.0);
    agent.dreaming.threshold.relative = Some(0.8);
    agent.search.enabled = Some(false);
    agent.search.model = Some("agent-search".into());
    agent.search.timeout_ms = Some(1000);
    agent.search.max_summary_chars = Some(200);
    agent.search.min_entity_hits = Some(4);
    agent.search.top_k_events = Some(10);
    agent.search.context_turns = Some(2);
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
    // memory_md_path is always global (design doc: "globally shared single rules file")
    assert_eq!(
        merged.storage.memory_md_path.as_deref(),
        Some("global/MEMORY.md")
    );
}

// --- Partial agent override: some fields override, rest inherit ---

#[test]
fn test_merge_memory_partial_override() {
    let global = make_global_memory();
    let mut agent = MemoryConfig::default();
    agent.mining.max_events_per_session = Some(5);
    agent.dreaming.threshold.absolute = Some(3.0);
    agent.search.timeout_ms = Some(1000);
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
    let mut global = MemoryConfig::default();
    global.mining.enabled = Some(false);
    global.search.enabled = Some(false);
    let mut agent = MemoryConfig::default();
    agent.mining.enabled = Some(true);
    agent.search.enabled = Some(true);
    let merged = global.merge_overrides(&agent);
    assert_eq!(merged.mining.enabled, Some(true));
    assert_eq!(merged.search.enabled, Some(true));
}

// --- Dreaming sub-config merge: scoring weights ---

#[test]
fn test_merge_memory_dreaming_scoring_override() {
    let global = make_global_memory();
    let mut agent = MemoryConfig::default();
    agent.dreaming.scoring.frequency_weight = Some(3.0);
    let merged = global.merge_overrides(&agent);
    // frequency_weight overridden
    assert_eq!(merged.dreaming.scoring.frequency_weight, Some(3.0));
    // Other scoring weights inherit global defaults (not overridden by agent)
    assert_eq!(merged.dreaming.scoring.recency_weight, Some(0.5));
    assert_eq!(merged.dreaming.scoring.explicitness_weight, Some(1.5));
}

// --- memory_md_path is always global (design doc: globally shared single rules file) ---

#[test]
fn test_merge_memory_md_path_ignores_agent_override() {
    let global = make_global_memory();
    let mut agent = MemoryConfig::default();
    agent.storage.db_path = Some("agent/db.sqlite".into());
    agent.storage.memory_md_path = Some("agent/NOTES.md".into());
    let merged = global.merge_overrides(&agent);

    // db_path overridden per-agent
    assert_eq!(merged.storage.db_path.as_deref(), Some("agent/db.sqlite"));
    // memory_md_path always global — agent override is ignored
    assert_eq!(
        merged.storage.memory_md_path.as_deref(),
        Some("global/MEMORY.md")
    );
}

// --- from_single with memory ---

#[test]
fn test_from_single_with_memory_config() {
    let mut memory = MemoryConfig::default();
    memory.search.enabled = Some(true);
    memory.search.timeout_ms = Some(10000);
    let config = AgentConfig {
        id: "test-agent".to_string(),
        memory: Some(memory),
        ..Default::default()
    };
    let resolved = from_single(config, "<test>", None).unwrap();
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
    let resolved = from_single(config, "<test>", None).unwrap();
    assert_eq!(resolved.memory, MemoryConfig::default());
}

// ------------------------------------------------------------------
// from_single vs merge consistency: subagent defaults
// ------------------------------------------------------------------

/// Verify that `from_single` and `merge` produce consistent subagent
/// defaults when given equivalent input. Both paths must yield the same
/// `require_agent_id`, `max_spawn_depth`, and `max_children` values.
#[test]
fn test_from_single_merge_consistency_subagent_defaults() {
    // from_single path: single config with all subagent fields None
    let single_config = AgentConfig {
        id: "consistency-agent".to_string(),
        subagents: SubagentsConfig::default(),
        ..Default::default()
    };
    let from_single_result = from_single(single_config, "<test>", None).unwrap();

    // merge path: project has empty id (so user id wins), all subagent fields None
    let project_config = AgentConfig {
        id: String::new(),
        subagents: SubagentsConfig::default(),
        ..Default::default()
    };
    let user_config = AgentConfig {
        id: "consistency-agent".to_string(),
        subagents: SubagentsConfig::default(),
        ..Default::default()
    };
    let merge_result = merge(project_config, user_config, "<test>", None).unwrap();

    // Subagent defaults must be identical across both paths
    assert_eq!(
        from_single_result.subagents.require_agent_id, merge_result.subagents.require_agent_id,
        "require_agent_id must match between from_single and merge"
    );
    assert_eq!(
        from_single_result.subagents.max_spawn_depth, merge_result.subagents.max_spawn_depth,
        "max_spawn_depth must match between from_single and merge"
    );
    assert_eq!(
        from_single_result.subagents.max_children, merge_result.subagents.max_children,
        "max_children must match between from_single and merge"
    );

    // Both should have the canonical defaults
    assert_eq!(from_single_result.subagents.require_agent_id, Some(false));
    assert_eq!(from_single_result.subagents.max_spawn_depth, Some(1));
    assert_eq!(from_single_result.subagents.max_children, Some(5));
}

/// Verify that from_single preserves explicit subagent values and merge
/// with matching explicit values produces the same result.
#[test]
fn test_from_single_merge_consistency_explicit_values() {
    let subagents = SubagentsConfig {
        require_agent_id: Some(true),
        max_spawn_depth: Some(3),
        max_children: Some(10),
        ..Default::default()
    };

    let from_single_result = from_single(
        AgentConfig {
            id: "explicit-agent".to_string(),
            subagents: subagents.clone(),
            ..Default::default()
        },
        "<test>",
        None,
    )
    .unwrap();

    let merge_result = merge(
        AgentConfig {
            id: String::new(),
            subagents: SubagentsConfig::default(),
            ..Default::default()
        },
        AgentConfig {
            id: "explicit-agent".to_string(),
            subagents,
            ..Default::default()
        },
        "<test>",
        None,
    )
    .unwrap();

    assert_eq!(
        from_single_result.subagents.require_agent_id,
        merge_result.subagents.require_agent_id
    );
    assert_eq!(
        from_single_result.subagents.max_spawn_depth,
        merge_result.subagents.max_spawn_depth
    );
    assert_eq!(
        from_single_result.subagents.max_children,
        merge_result.subagents.max_children
    );
}

// ------------------------------------------------------------------
// merge_subagents: timeout field merge
// ------------------------------------------------------------------

#[test]
fn test_merge_project_timeout_overrides_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            timeout: Some(60),
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            timeout: Some(120),
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.timeout, Some(60));
}

#[test]
fn test_merge_project_timeout_none_falls_back_to_user() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            timeout: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            timeout: Some(120),
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert_eq!(resolved.subagents.timeout, Some(120));
}

#[test]
fn test_merge_both_timeout_none_remains_none() {
    let project = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            timeout: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let user = AgentConfig {
        id: "test-agent".to_string(),
        subagents: SubagentsConfig {
            timeout: None,
            ..Default::default()
        },
        ..Default::default()
    };
    let resolved = merge(project, user, "<test>", None).unwrap();
    assert!(resolved.subagents.timeout.is_none());
}
