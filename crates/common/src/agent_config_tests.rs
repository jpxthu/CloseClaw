//! Tests for the SubagentsConfig type (serde / Default).
//!
//! Migrated from `closeclaw_config::agents::config_types` alongside the type
//! definitions in `agent_config.rs` (issue #3344).

use super::*;

// ── SubagentsConfig timeout tests ─────────────────────────────────

#[test]
fn test_subagents_config_timeout_serialize() {
    let config = SubagentsConfig {
        timeout: Some(120),
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"timeout\""));
    assert!(json.contains("120"));
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

// ── SubagentsConfig Default semantics ─────────────────────────────

#[test]
#[allow(deprecated)] // default_child_agent is deprecated; default value is still None
fn test_subagents_config_defaults() {
    let config = SubagentsConfig::default();

    assert_eq!(config.allow_agents, vec!["*"]);
    assert_eq!(config.require_agent_id, None);
    assert_eq!(config.max_spawn_depth, None);
    assert_eq!(config.max_children, None);
    assert!(config.default_child_agent.is_none());
    assert!(config.model.is_none());
}
