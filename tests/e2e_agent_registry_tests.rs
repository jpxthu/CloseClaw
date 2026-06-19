//! E2E integration tests for Agent Registry.
//!
//! Exercises the public entry point `create_registry` → `populate` → `get` →
//! `reload` lifecycle, simulating daemon startup fill, runtime query,
//! and config hot-reload.

use closeclaw::agent::config::SubagentsConfig;
use closeclaw::agent::registry::create_registry;
use closeclaw::config::agents::{ConfigSource, ResolvedAgentConfig};
use closeclaw::session::bootstrap::BootstrapMode;

/// Helper: build a minimal `ResolvedAgentConfig` for E2E tests.
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

/// Startup fill → query hit and miss.
#[tokio::test]
async fn test_populate_then_get() {
    let registry = create_registry();
    let configs = vec![make_config("agent-a"), make_config("agent-b")];

    registry.populate(configs);

    // Hit: agent-a should be found
    let a = registry.get("agent-a");
    assert!(a.is_some(), "agent-a should be found after populate");
    assert_eq!(a.unwrap().id, "agent-a");

    // Hit: agent-b should be found
    let b = registry.get("agent-b");
    assert!(b.is_some(), "agent-b should be found after populate");
    assert_eq!(b.unwrap().id, "agent-b");

    // Miss: nonexistent agent returns None
    let missing = registry.get("nonexistent");
    assert!(
        missing.is_none(),
        "querying a missing id should return None"
    );
}

/// Hot-reload: old data cleared, new data effective.
#[tokio::test]
async fn test_hot_reload_replaces_data() {
    let registry = create_registry();

    // Populate with initial data.
    registry.populate(vec![make_config("old-agent")]);
    assert!(
        registry.get("old-agent").is_some(),
        "old-agent should exist before reload"
    );

    // Hot-reload with new set that excludes old-agent.
    registry.reload(vec![make_config("new-agent")]);

    assert!(
        registry.get("old-agent").is_none(),
        "old-agent should be gone after reload"
    );
    let new = registry.get("new-agent");
    assert!(new.is_some(), "new-agent should be present after reload");
    assert_eq!(new.unwrap().id, "new-agent");
}

/// Hot-reload: shared agents survive, removed agents gone, added agents appear.
#[tokio::test]
async fn test_hot_reload_partial_overlap() {
    let registry = create_registry();

    registry.populate(vec![make_config("keep"), make_config("drop")]);

    // Reload with "keep" retained, "drop" removed, "added" new.
    registry.reload(vec![make_config("keep"), make_config("added")]);

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

/// Empty populate does not panic.
#[tokio::test]
async fn test_populate_empty() {
    let registry = create_registry();

    registry.populate(vec![]);

    assert!(
        registry.get("anything").is_none(),
        "empty populate should leave registry empty"
    );
}

/// Populate with duplicate IDs: the last entry wins.
#[tokio::test]
async fn test_populate_duplicate_id_last_wins() {
    let registry = create_registry();

    let mut first = make_config("dup-agent");
    first.name = "first".to_string();
    let mut second = make_config("dup-agent");
    second.name = "second".to_string();

    registry.populate(vec![first, second]);

    let agent = registry.get("dup-agent");
    assert!(agent.is_some(), "dup-agent should exist");
    assert_eq!(
        agent.unwrap().name,
        "second",
        "later entry should overwrite earlier one"
    );
}

/// Registry starts empty after create_registry.
#[tokio::test]
async fn test_registry_starts_empty() {
    let registry = create_registry();

    assert!(
        registry.get("any-id").is_none(),
        "newly created registry should have no configs"
    );
}
