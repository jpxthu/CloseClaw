//! Tests for the agent config type family (serde / Default / effective_*).
//!
//! Migrated from `closeclaw_config::agents::config_types` alongside the type
//! definitions in `agent_config.rs` (issue #3344).

use super::*;
use crate::{BootstrapMode, MemoryConfig};

// ── SubagentsConfig timeout tests ─────────────────────────────────

#[test]
fn test_subagents_config_timeout_serialize() {
    let config = SubagentsConfig {
        timeout: Some(120),
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"timeout\""));
    let round_tripped: SubagentsConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(round_tripped.timeout, Some(120));
}

#[test]
fn test_subagents_config_timeout_deserialize() {
    let json = r#"{"timeout": 120}"#;
    let config: SubagentsConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.timeout, Some(120));
}

#[test]
fn test_subagents_config_default_timeout_is_none() {
    let config = SubagentsConfig::default();
    assert!(config.timeout.is_none());
}

#[test]
fn test_subagents_config_timeout_none_skip() {
    let config = SubagentsConfig {
        timeout: None,
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(!json.contains("\"timeout\""));
}

// ── SubagentsConfig camelCase serde renames ───────────────────────

#[test]
fn test_subagents_config_camel_case_field_names() {
    // Every Option/Vec field must deserialize from its camelCase JSON key,
    // including the explicit renames (timeoutWarning / timeoutNotifyIntervalRatio).
    let json = r#"{
        "allowAgents": ["agent-a"],
        "requireAgentId": true,
        "maxSpawnDepth": 3,
        "maxChildren": 10,
        "timeout": 120,
        "timeoutWarning": 60,
        "timeoutNotifyIntervalRatio": 0.25
    }"#;
    let config: SubagentsConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.allow_agents, vec!["agent-a".to_string()]);
    assert_eq!(config.require_agent_id, Some(true));
    assert_eq!(config.max_spawn_depth, Some(3));
    assert_eq!(config.max_children, Some(10));
    assert_eq!(config.timeout, Some(120));
    assert_eq!(config.timeout_warning, Some(60));
    assert_eq!(config.timeout_notify_interval_ratio, Some(0.25));
}

#[test]
fn test_subagents_config_timeout_fields_serialize_renames() {
    let config = SubagentsConfig {
        timeout: Some(120),
        timeout_warning: Some(60),
        timeout_notify_interval_ratio: Some(0.25),
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"timeout\""));
    assert!(json.contains("\"timeoutWarning\""));
    assert!(json.contains("\"timeoutNotifyIntervalRatio\""));
}

// ── SubagentsConfig Default semantics ─────────────────────────────

#[test]
#[allow(deprecated)] // default_child_agent is deprecated; default value is still None
fn test_subagents_config_defaults() {
    let config = SubagentsConfig::default();

    assert_eq!(config.allow_agents, vec!["*"]);
    assert_eq!(config.require_agent_id, None);
    assert_eq!(config.max_spawn_depth, None);
    assert_eq!(config.max_children, None);
    assert_eq!(config.timeout_warning, None);
    assert_eq!(config.timeout_notify_interval_ratio, None);
    assert!(config.default_child_agent.is_none());
    assert!(config.model.is_none());
}

// ── ResolvedAgentConfig::effective_* whitelist semantics ──────────

/// Base resolved config for `effective_*` tests; individual tests overwrite
/// the list fields under test.
fn resolved_fixture() -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: "test-agent".to_string(),
        name: "test-agent".to_string(),
        parent_id: None,
        model: None,
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        no_bootstrap: false,
        skills: vec!["*".to_string()],
        tools: vec!["*".to_string()],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        memory: MemoryConfig::default(),
        memory_configured: false,
        hooks: vec![],
        parallel_tool_calls: true,
    }
}

#[test]
fn test_effective_skills_wildcard_and_empty_mean_unrestricted() {
    let mut config = resolved_fixture();
    // `["*"]` wildcard → no filtering.
    config.skills = vec!["*".to_string()];
    assert_eq!(config.effective_skills(), None);
    // Empty list → no filtering either.
    config.skills = vec![];
    assert_eq!(config.effective_skills(), None);
    // Concrete list → returned verbatim as the whitelist.
    config.skills = vec!["github".to_string(), "search".to_string()];
    assert_eq!(
        config.effective_skills(),
        Some(vec!["github".to_string(), "search".to_string()])
    );
}

#[test]
fn test_effective_tools_wildcard_and_empty_mean_unrestricted() {
    let mut config = resolved_fixture();
    config.tools = vec!["*".to_string()];
    assert_eq!(config.effective_tools(), None);
    config.tools = vec![];
    assert_eq!(config.effective_tools(), None);
    config.tools = vec!["read".to_string(), "grep".to_string()];
    assert_eq!(
        config.effective_tools(),
        Some(vec!["read".to_string(), "grep".to_string()])
    );
}

#[test]
fn test_effective_disallowed_tools_empty_none_non_empty_some() {
    let mut config = resolved_fixture();
    // Empty blacklist → nothing blocked.
    assert_eq!(config.effective_disallowed_tools(), None);
    config.disallowed_tools = vec!["shell".to_string()];
    assert_eq!(
        config.effective_disallowed_tools(),
        Some(vec!["shell".to_string()])
    );
    // Blacklist and whitelist are independent inputs: an unrestricted
    // whitelist (wildcard) still coexists with an active blacklist, which
    // is the precondition for consumers to apply the blacklist on top.
    config.tools = vec!["*".to_string()];
    assert_eq!(config.effective_tools(), None);
    assert_eq!(
        config.effective_disallowed_tools(),
        Some(vec!["shell".to_string()])
    );
}

// ── Resolved product carries no source marker (issue #3344) ───────

// Compile-level confirmation: `source` / `ConfigSource` were removed from
// the resolved product (field alignment against
// docs/design/common/shared-types.md). Exhaustive destructuring without
// `..` fails to compile (E0027, "pattern does not mention field") if a
// `source` field — or any field outside the authoritative schema — is
// ever reintroduced.
fn assert_authoritative_field_set(
    ResolvedAgentConfig {
        id: _,
        name: _,
        parent_id: _,
        model: _,
        workspace: _,
        agent_dir: _,
        bootstrap_mode: _,
        no_bootstrap: _,
        skills: _,
        tools: _,
        disallowed_tools: _,
        subagents: _,
        memory: _,
        memory_configured: _,
        hooks: _,
        parallel_tool_calls: _,
    }: ResolvedAgentConfig,
) {
}

#[test]
fn resolved_product_carries_no_source_marker() {
    assert_authoritative_field_set(resolved_fixture());
}
