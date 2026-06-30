use crate::config::SubagentsConfig;
use crate::registry::AgentRegistry;
use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};
use closeclaw_session::bootstrap::BootstrapMode;

// ---- Construction tests ----

#[test]
fn test_new_construction() {
    let registry = AgentRegistry::new();
    // After construction the config map must be empty.
    assert!(
        registry.get("any-id").is_none(),
        "newly created registry should have no configs"
    );
}

#[test]
fn test_default_trait() {
    let registry = AgentRegistry::default();
    // Default() delegates to new(), should be empty.
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
        memory: None,
        source: ConfigSource::User,
    }
}

#[test]
fn test_populate_and_get() {
    let registry = AgentRegistry::new();
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
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("existing")]);

    let result = registry.get("nonexistent");
    assert!(result.is_none(), "querying a missing id should return None");
}

#[test]
fn test_reload() {
    let registry = AgentRegistry::new();

    // Populate with old data.
    registry.populate(vec![make_config("old-agent")]);
    assert!(
        registry.get("old-agent").is_some(),
        "old-agent should exist before reload"
    );

    // Reload with new data that does NOT include old-agent.
    registry.reload(vec![make_config("new-agent")]);

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
    let registry = AgentRegistry::new();

    // Should not panic on empty input.
    registry.populate(vec![]);

    assert!(
        registry.get("anything").is_none(),
        "empty populate should leave registry empty"
    );
}

#[test]
fn test_reload_preserves_existing() {
    let registry = AgentRegistry::new();

    registry.populate(vec![make_config("keep"), make_config("drop")]);

    // Reload: only "keep" and a new agent "added" exist.
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

// ---- DashMap reference return type tests ----

#[test]
fn test_get_returns_valid_reference() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("agent-1")]);

    // The DashMap Ref implements Deref<Target = ResolvedAgentConfig>,
    // so we can access fields directly.
    let agent = registry.get("agent-1").expect("agent-1 should exist");
    assert_eq!(agent.id, "agent-1");
    assert_eq!(agent.name, "agent-1");
    assert_eq!(agent.bootstrap_mode, BootstrapMode::Full);
    assert!(agent.skills.is_empty());
    assert!(agent.tools.is_empty());
    assert!(agent.disallowed_tools.is_empty());
}

#[test]
fn test_get_reference_sees_populated_data() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("agent-x");
    cfg.skills = vec!["skill-a".into(), "skill-b".into()];
    cfg.tools = vec!["tool-1".into()];
    cfg.disallowed_tools = vec!["tool-2".into()];
    registry.populate(vec![cfg]);

    let agent = registry.get("agent-x").expect("agent-x should exist");
    assert_eq!(agent.skills, vec!["skill-a", "skill-b"]);
    assert_eq!(agent.tools, vec!["tool-1"]);
    assert_eq!(agent.disallowed_tools, vec!["tool-2"]);
}

#[test]
fn test_get_reference_sees_reload_data() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("old")]);

    // Verify old data is visible through reference
    {
        let old = registry.get("old").expect("old should exist");
        assert_eq!(old.id, "old");
    } // DashMap Ref dropped here

    // Reload with new config
    let mut new_cfg = make_config("old");
    new_cfg.skills = vec!["updated-skill".into()];
    registry.reload(vec![new_cfg]);

    // Reference should see updated data
    let old = registry.get("old").expect("old should exist after reload");
    assert_eq!(old.skills, vec!["updated-skill"]);
}

// ---- query_bootstrap_mode tests ----

#[test]
fn test_query_bootstrap_mode_full() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("agent-full");
    cfg.bootstrap_mode = BootstrapMode::Full;
    registry.populate(vec![cfg]);

    let mode = registry.query_bootstrap_mode("agent-full");
    assert_eq!(mode, Some(BootstrapMode::Full));
}

#[test]
fn test_query_bootstrap_mode_minimal() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("agent-minimal");
    cfg.bootstrap_mode = BootstrapMode::Minimal;
    registry.populate(vec![cfg]);

    let mode = registry.query_bootstrap_mode("agent-minimal");
    assert_eq!(mode, Some(BootstrapMode::Minimal));
}

#[test]
fn test_query_bootstrap_mode_not_found() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("existing")]);

    let mode = registry.query_bootstrap_mode("nonexistent");
    assert_eq!(mode, None);
}

#[test]
fn test_query_bootstrap_mode_empty_registry() {
    let registry = AgentRegistry::new();
    let mode = registry.query_bootstrap_mode("any-id");
    assert_eq!(mode, None);
}

// ---- Concurrent access tests ----

#[test]
fn test_concurrent_get_and_populate() {
    use std::sync::Arc;
    use std::thread;

    let registry = Arc::new(AgentRegistry::new());

    // Spawn threads that read and write concurrently
    let mut handles = vec![];

    // Writer thread: populate with configs
    let reg_write = Arc::clone(&registry);
    handles.push(thread::spawn(move || {
        for i in 0..50 {
            let cfg = make_config(&format!("writer-{}", i));
            reg_write.populate(vec![cfg]);
        }
    }));

    // Reader threads: query configs
    for _t in 0..5 {
        let reg_read = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            for _i in 0..50 {
                let id = format!("writer-{}", _i);
                let _result = reg_read.get(&id);
                // Just verify no panic — DashMap handles concurrent access
            }
        }));
    }

    for h in handles {
        h.join().expect("thread should not panic");
    }

    // After all writers, at least some configs should be present
    assert!(!registry.get("writer-0").is_none() || !registry.get("writer-49").is_none());
}

#[test]
fn test_concurrent_get_and_reload() {
    use std::sync::Arc;
    use std::thread;

    let registry = Arc::new(AgentRegistry::new());
    registry.populate(vec![make_config("initial")]);

    let mut handles = vec![];

    // Reload thread: repeatedly replace all configs
    let reg_write = Arc::clone(&registry);
    handles.push(thread::spawn(move || {
        for i in 0..30 {
            let configs: Vec<_> = (0..10)
                .map(|j| make_config(&format!("reload-{}-{}", i, j)))
                .collect();
            reg_write.reload(configs);
        }
    }));

    // Reader threads
    for _ in 0..5 {
        let reg_read = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            for _i in 0..50 {
                // Query any ID — should never panic
                let _ = reg_read.get("reload-0-0");
                let _ = reg_read.query_bootstrap_mode("any");
            }
        }));
    }

    for h in handles {
        h.join().expect("thread should not panic");
    }
}

#[test]
fn test_concurrent_query_bootstrap_mode() {
    use std::sync::Arc;
    use std::thread;

    let registry = Arc::new(AgentRegistry::new());
    let mut cfg_full = make_config("full-agent");
    cfg_full.bootstrap_mode = BootstrapMode::Full;
    let mut cfg_min = make_config("min-agent");
    cfg_min.bootstrap_mode = BootstrapMode::Minimal;
    registry.populate(vec![cfg_full, cfg_min]);

    let mut handles = vec![];
    for _ in 0..10 {
        let reg = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                let mode_full = reg.query_bootstrap_mode("full-agent");
                assert_eq!(mode_full, Some(BootstrapMode::Full));
                let mode_min = reg.query_bootstrap_mode("min-agent");
                assert_eq!(mode_min, Some(BootstrapMode::Minimal));
                let mode_none = reg.query_bootstrap_mode("missing");
                assert_eq!(mode_none, None);
            }
        }));
    }

    for h in handles {
        h.join().expect("thread should not panic");
    }
}
