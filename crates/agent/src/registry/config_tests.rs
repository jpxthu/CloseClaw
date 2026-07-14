use crate::config::MemoryConfig;
use crate::config::SubagentsConfig;
use crate::registry::AgentRegistry;
use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::{ConfigSource, ModelSpec, ResolvedAgentConfig};

// Trait imports for AgentLookup / AgentSkillsQuery / AgentToolsConfigQuery / AgentConfigLookup tests
use crate::lookup::{AgentConfigLookup, AgentLookup};
use crate::skills_query::AgentSkillsQuery;
use crate::tools_config_query::AgentToolsConfigQuery;

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
        memory: MemoryConfig::default(),
        hooks: Vec::new(),
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

// ---- AgentLookup trait tests ----

#[tokio::test]
async fn test_agent_lookup_get_agent_model() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("agent-model");
    cfg.model = Some(ModelSpec::single("gpt-4o"));
    registry.populate(vec![cfg]);

    let model = registry.get_agent_model("agent-model").await;
    assert_eq!(model, Some(ModelSpec::single("gpt-4o")));
}

#[tokio::test]
async fn test_agent_lookup_get_agent_model_no_model() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("no-model")]);

    let model = registry.get_agent_model("no-model").await;
    assert_eq!(model, None);
}

#[tokio::test]
async fn test_agent_lookup_get_agent_model_not_found() {
    let registry = AgentRegistry::new();
    registry.populate(vec![]);

    let model = registry.get_agent_model("missing").await;
    assert_eq!(model, None);
}

#[tokio::test]
async fn test_agent_lookup_agent_exists() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("exists")]);

    assert!(registry.agent_exists("exists").await);
    assert!(!registry.agent_exists("missing").await);
}

// ---- AgentSkillsQuery trait tests ----

#[test]
fn test_agent_skills_query_wildcard() {
    let registry = AgentRegistry::new();
    // Default skills = ["*"]
    registry.populate(vec![make_config("wildcard-agent")]);

    // Wildcard → effective_skills() returns None → get_agent_skills returns None
    let skills = registry.get_agent_skills("wildcard-agent");
    assert_eq!(skills, None);
}

#[test]
fn test_agent_skills_query_specific_skills() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("specific-agent");
    cfg.skills = vec!["coding".to_string(), "search".to_string()];
    registry.populate(vec![cfg]);

    let skills = registry.get_agent_skills("specific-agent");
    assert_eq!(
        skills,
        Some(vec!["coding".to_string(), "search".to_string()])
    );
}

#[test]
fn test_agent_skills_query_not_found() {
    let registry = AgentRegistry::new();
    let skills = registry.get_agent_skills("missing");
    assert_eq!(skills, None);
}

#[test]
fn test_agent_skills_query_empty_skills() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("empty-skills");
    cfg.skills = vec![];
    registry.populate(vec![cfg]);

    // Empty skills list is treated as wildcard (no filtering) → None
    let skills = registry.get_agent_skills("empty-skills");
    assert_eq!(skills, None);
}

// ---- AgentToolsConfigQuery trait tests ----

#[tokio::test]
async fn test_agent_tools_query_wildcard_tools() {
    let registry = AgentRegistry::new();
    // Default tools = ["*"]
    registry.populate(vec![make_config("wildcard-tools")]);

    let config = registry.get_agent_tools_config("wildcard-tools").await;
    assert!(config.is_some());
    let cfg = config.unwrap();
    // Wildcard → tools=None (no filtering)
    assert_eq!(cfg.tools, None);
    // Default disallowed_tools = [] → None
    assert_eq!(cfg.disallowed_tools, None);
}

#[tokio::test]
async fn test_agent_tools_query_specific_tools() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("specific-tools");
    cfg.tools = vec!["read".to_string(), "write".to_string()];
    registry.populate(vec![cfg]);

    let config = registry.get_agent_tools_config("specific-tools").await;
    assert!(config.is_some());
    let cfg = config.unwrap();
    assert_eq!(
        cfg.tools,
        Some(vec!["read".to_string(), "write".to_string()])
    );
    assert_eq!(cfg.disallowed_tools, None);
}

#[tokio::test]
async fn test_agent_tools_query_with_disallowed() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("disallowed-tools");
    cfg.tools = vec!["read".to_string()];
    cfg.disallowed_tools = vec!["exec".to_string(), "web_search".to_string()];
    registry.populate(vec![cfg]);

    let config = registry.get_agent_tools_config("disallowed-tools").await;
    assert!(config.is_some());
    let cfg = config.unwrap();
    assert_eq!(cfg.tools, Some(vec!["read".to_string()]));
    assert_eq!(
        cfg.disallowed_tools,
        Some(vec!["exec".to_string(), "web_search".to_string()])
    );
}

#[tokio::test]
async fn test_agent_tools_query_not_found() {
    let registry = AgentRegistry::new();
    let config = registry.get_agent_tools_config("missing").await;
    assert!(config.is_none());
}

#[tokio::test]
async fn test_agent_tools_query_empty_disallowed_is_none() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("empty-disallowed");
    cfg.tools = vec!["read".to_string()];
    cfg.disallowed_tools = vec![];
    registry.populate(vec![cfg]);

    let config = registry.get_agent_tools_config("empty-disallowed").await;
    let cfg = config.unwrap();
    assert_eq!(cfg.disallowed_tools, None);
}

// ---- AgentConfigLookup trait tests ----

#[tokio::test]
async fn test_agent_config_lookup_with_model() {
    let registry = AgentRegistry::new();
    let mut subagents = SubagentsConfig::default();
    subagents.model = Some(ModelSpec::single("claude-3"));
    let mut cfg = make_config("config-lookup");
    cfg.subagents = subagents;
    registry.populate(vec![cfg]);

    let info = registry.lookup_agent_config("config-lookup").await;
    assert!(info.is_some());
    assert_eq!(
        info.unwrap().subagents_model,
        Some(ModelSpec::single("claude-3"))
    );
}

#[tokio::test]
async fn test_agent_config_lookup_no_model() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("no-model-lookup")]);

    let info = registry.lookup_agent_config("no-model-lookup").await;
    assert!(info.is_some());
    assert_eq!(info.unwrap().subagents_model, None);
}

#[tokio::test]
async fn test_agent_config_lookup_not_found() {
    let registry = AgentRegistry::new();
    let info = registry.lookup_agent_config("missing").await;
    assert!(info.is_none());
}

// ---- create_registry helper test ----

#[test]
fn test_create_registry_helper() {
    let registry = crate::registry::create_registry();
    // Should be a shared, empty registry
    assert!(registry.get("anything").is_none());
    // Populate and verify
    registry.populate(vec![make_config("helper-test")]);
    assert!(registry.get("helper-test").is_some());
}
