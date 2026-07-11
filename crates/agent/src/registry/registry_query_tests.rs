//! Tests for `AgentRegistryQuery` combined trait and `get_agent_workspace`.
//!
//! Verifies that `AgentRegistry` correctly implements all three supertraits
//! (AgentLookup + AgentSkillsQuery + AgentToolsConfigQuery) and that the
//! combined `AgentRegistryQuery` trait is usable through `Arc<dyn ...>`.

use crate::config::MemoryConfig;
use crate::config::SubagentsConfig;
use crate::lookup::{AgentLookup, AgentRegistryQuery};
use crate::registry::AgentRegistry;
use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::{ConfigSource, ModelSpec, ResolvedAgentConfig};
use std::path::PathBuf;
use std::sync::Arc;

/// Helper: build a `ResolvedAgentConfig` for tests.
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
        source: ConfigSource::User,
    }
}

// ── AgentLookup::get_agent_workspace tests ─────────────────────────────────

#[tokio::test]
async fn test_get_agent_workspace_with_value() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("ws-agent");
    cfg.workspace = Some(PathBuf::from("/home/user/project"));
    registry.populate(vec![cfg]);

    let ws = registry.get_agent_workspace("ws-agent").await;
    assert_eq!(ws, Some(PathBuf::from("/home/user/project")));
}

#[tokio::test]
async fn test_get_agent_workspace_none() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("no-ws")]);

    let ws = registry.get_agent_workspace("no-ws").await;
    assert_eq!(ws, None);
}

#[tokio::test]
async fn test_get_agent_workspace_not_found() {
    let registry = AgentRegistry::new();
    let ws = registry.get_agent_workspace("missing").await;
    assert_eq!(ws, None);
}

#[tokio::test]
async fn test_get_agent_workspace_empty_registry() {
    let registry = AgentRegistry::new();
    let ws = registry.get_agent_workspace("any").await;
    assert_eq!(ws, None);
}

// ── Combined trait: Arc<dyn AgentRegistryQuery> ─────────────────────────────

#[tokio::test]
async fn test_combined_trait_arc_dispatch_model() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("model-agent");
    cfg.model = Some(ModelSpec::single("gpt-4o"));
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let model = arc_registry.get_agent_model("model-agent").await;
    assert_eq!(model, Some(ModelSpec::single("gpt-4o")));
}

#[tokio::test]
async fn test_combined_trait_arc_dispatch_workspace() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("ws-agent");
    cfg.workspace = Some(PathBuf::from("/opt/work"));
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let ws = arc_registry.get_agent_workspace("ws-agent").await;
    assert_eq!(ws, Some(PathBuf::from("/opt/work")));
}

#[tokio::test]
async fn test_combined_trait_arc_dispatch_skills() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("skill-agent");
    cfg.skills = vec!["coding".to_string(), "search".to_string()];
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let skills = arc_registry.get_agent_skills("skill-agent");
    assert_eq!(
        skills,
        Some(vec!["coding".to_string(), "search".to_string()])
    );
}

#[tokio::test]
async fn test_combined_trait_arc_dispatch_tools() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("tool-agent");
    cfg.tools = vec!["read".to_string()];
    cfg.disallowed_tools = vec!["exec".to_string()];
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let tools_cfg = arc_registry.get_agent_tools_config("tool-agent").await;
    assert!(tools_cfg.is_some());
    let tc = tools_cfg.unwrap();
    assert_eq!(tc.tools, Some(vec!["read".to_string()]));
    assert_eq!(tc.disallowed_tools, Some(vec!["exec".to_string()]));
}

#[tokio::test]
async fn test_combined_trait_arc_dispatch_exists() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("existing")]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    assert!(arc_registry.agent_exists("existing").await);
    assert!(!arc_registry.agent_exists("missing").await);
}

#[tokio::test]
async fn test_combined_trait_arc_dispatch_bootstrap_mode() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("min-agent");
    cfg.bootstrap_mode = BootstrapMode::Minimal;
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let mode = arc_registry.query_bootstrap_mode("min-agent").await;
    assert_eq!(mode, Some(BootstrapMode::Minimal));
}

// ── Combined trait: wildcard skills ─────────────────────────────────────────

#[tokio::test]
async fn test_combined_trait_arc_wildcard_skills() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("wildcard-skill");
    cfg.skills = vec!["*".to_string()];
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let skills = arc_registry.get_agent_skills("wildcard-skill");
    // Wildcard → effective_skills returns None → get_agent_skills returns None
    assert_eq!(skills, None);
}

#[tokio::test]
async fn test_combined_trait_arc_empty_skills() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("empty-skill");
    cfg.skills = vec![];
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let skills = arc_registry.get_agent_skills("empty-skill");
    // Empty → treated as wildcard → None
    assert_eq!(skills, None);
}

// ── Combined trait: wildcard tools ──────────────────────────────────────────

#[tokio::test]
async fn test_combined_trait_arc_wildcard_tools() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("wildcard-tool");
    cfg.tools = vec!["*".to_string()];
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let tc = arc_registry
        .get_agent_tools_config("wildcard-tool")
        .await
        .unwrap();
    // Wildcard → tools=None
    assert_eq!(tc.tools, None);
}

#[tokio::test]
async fn test_combined_trait_arc_empty_tools() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("empty-tool");
    cfg.tools = vec![];
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    let tc = arc_registry
        .get_agent_tools_config("empty-tool")
        .await
        .unwrap();
    // Empty → treated as wildcard → None
    assert_eq!(tc.tools, None);
}

// ── Combined trait: agent not found returns correct defaults ────────────────

#[tokio::test]
async fn test_combined_trait_arc_missing_agent() {
    let registry = AgentRegistry::new();
    registry.populate(vec![make_config("existing")]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);
    assert!(!arc_registry.agent_exists("missing").await);
    assert_eq!(arc_registry.get_agent_model("missing").await, None);
    assert_eq!(arc_registry.get_agent_workspace("missing").await, None);
    assert_eq!(arc_registry.get_agent_skills("missing"), None);
    assert_eq!(arc_registry.get_agent_tools_config("missing").await, None);
    assert_eq!(arc_registry.query_bootstrap_mode("missing").await, None);
}

// ── Combined trait: all three supertraits coexist ───────────────────────────

#[tokio::test]
async fn test_combined_trait_all_queries_on_same_agent() {
    let registry = AgentRegistry::new();
    let mut cfg = make_config("full-agent");
    cfg.model = Some(ModelSpec::single("claude-3"));
    cfg.workspace = Some(PathBuf::from("/workspace"));
    cfg.skills = vec!["coding".to_string()];
    cfg.tools = vec!["read".to_string()];
    cfg.disallowed_tools = vec!["exec".to_string()];
    cfg.bootstrap_mode = BootstrapMode::Minimal;
    registry.populate(vec![cfg]);

    let arc_registry: Arc<dyn AgentRegistryQuery> = Arc::new(registry);

    assert_eq!(
        arc_registry.get_agent_model("full-agent").await,
        Some(ModelSpec::single("claude-3"))
    );
    assert_eq!(
        arc_registry.get_agent_workspace("full-agent").await,
        Some(PathBuf::from("/workspace"))
    );
    assert_eq!(
        arc_registry.get_agent_skills("full-agent"),
        Some(vec!["coding".to_string()])
    );
    let tc = arc_registry
        .get_agent_tools_config("full-agent")
        .await
        .unwrap();
    assert_eq!(tc.tools, Some(vec!["read".to_string()]));
    assert_eq!(tc.disallowed_tools, Some(vec!["exec".to_string()]));
    assert_eq!(
        arc_registry.query_bootstrap_mode("full-agent").await,
        Some(BootstrapMode::Minimal)
    );
}
