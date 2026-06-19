use crate::agent::config::SubagentsConfig;
use crate::agent::registry::AgentRegistry;
use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
use crate::session::bootstrap::BootstrapMode;

// ---- Construction tests ----

#[test]
fn test_new_construction() {
    let registry = AgentRegistry::new(30);
    // After construction the config map must be empty.
    assert!(
        registry.get("any-id").is_none(),
        "newly created registry should have no configs"
    );
}

#[test]
fn test_new_with_graceful_shutdown() {
    let registry = AgentRegistry::new_with_graceful_shutdown(30);
    // Must construct successfully and start empty.
    assert!(
        registry.get("any-id").is_none(),
        "registry created via new_with_graceful_shutdown should have no configs"
    );
}

#[test]
fn test_default_trait() {
    let registry = AgentRegistry::default();
    // Default() delegates to new_with_graceful_shutdown, should be empty.
    assert!(
        registry.get("any-id").is_none(),
        "default registry should have no configs"
    );
}

/// Helper: build a minimal `ResolvedAgentConfig` for tests.
fn make_config(id: &str) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: None,
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        source: ConfigSource::User,
    }
}

#[test]
fn test_populate_and_get() {
    let registry = AgentRegistry::new(30);
    let configs = vec![make_config("agent-a"), make_config("agent-b")];

    registry.populate(configs);

    let a = registry.get("agent-a");
    assert!(a.is_some(), "agent-a should be found after populate");
    assert_eq!(a.unwrap().id, "agent-a");

    let b = registry.get("agent-b");
    assert!(b.is_some(), "agent-b should be found after populate");
    assert_eq!(b.unwrap().id, "agent-b");
}

#[test]
fn test_get_not_found() {
    let registry = AgentRegistry::new(30);
    registry.populate(vec![make_config("existing")]);

    let result = registry.get("nonexistent");
    assert!(result.is_none(), "querying a missing id should return None");
}

#[test]
fn test_reload_config() {
    let registry = AgentRegistry::new(30);

    // Populate with old data.
    registry.populate(vec![make_config("old-agent")]);
    assert!(
        registry.get("old-agent").is_some(),
        "old-agent should exist before reload"
    );

    // Reload with new data that does NOT include old-agent.
    registry.reload_config(vec![make_config("new-agent")]);

    assert!(
        registry.get("old-agent").is_none(),
        "old-agent should be gone after reload"
    );
    let new = registry.get("new-agent");
    assert!(new.is_some(), "new-agent should be present after reload");
    assert_eq!(new.unwrap().id, "new-agent");
}

#[test]
fn test_populate_empty() {
    let registry = AgentRegistry::new(30);

    // Should not panic on empty input.
    registry.populate(vec![]);

    assert!(
        registry.get("anything").is_none(),
        "empty populate should leave registry empty"
    );
}

#[test]
fn test_reload_config_preserves_existing() {
    let registry = AgentRegistry::new(30);

    registry.populate(vec![make_config("keep"), make_config("drop")]);

    // Reload: only "keep" and a new agent "added" exist.
    registry.reload_config(vec![make_config("keep"), make_config("added")]);

    assert!(
        registry.get("keep").is_some(),
        "'keep' should survive reload"
    );
    assert!(
        registry.get("drop").is_none(),
        "'drop' should be removed by reload"
    );
    assert!(
        registry.get("added").is_some(),
        "'added' should be present after reload"
    );
}
