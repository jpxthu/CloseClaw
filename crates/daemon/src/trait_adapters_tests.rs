//! Unit tests for `trait_adapters.rs`.
//!
//! Verifies the daemon mapping `ResolvedAgentConfig.no_bootstrap` →
//! `SpawnTargetAgentConfig` (issue #3475 spawn chain). Split from
//! `trait_adapters.rs` (STANDARDS.md §2: new files must not inline
//! `#[cfg(test)]`); mounted via `#[path]` (bootstrap.rs precedent).

use super::*;

fn resolved_agent_config(no_bootstrap: bool) -> closeclaw_common::ResolvedAgentConfig {
    closeclaw_common::ResolvedAgentConfig {
        id: "child-agent".to_string(),
        name: "child-agent".to_string(),
        parent_id: None,
        model: None,
        workspace: None,
        agent_dir: None,
        bootstrap_mode: closeclaw_common::BootstrapMode::Full,
        no_bootstrap,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: Default::default(),
        memory: Default::default(),
        memory_configured: false,
        hooks: Vec::new(),
        parallel_tool_calls: true,
    }
}

/// Mapping must carry `ResolvedAgentConfig.no_bootstrap` into the
/// spawn-time view — daemon is the single-point definition; dropping
/// the field would silently re-enable bootstrap loading for
/// noBootstrap agents.
#[test]
fn test_spawn_target_agent_config_maps_no_bootstrap_true() {
    let cfg = resolved_agent_config(true);
    let view = spawn_target_agent_config(&cfg);
    assert!(
        view.no_bootstrap,
        "no_bootstrap=true must survive the mapping"
    );
}

#[test]
fn test_spawn_target_agent_config_maps_no_bootstrap_false() {
    let cfg = resolved_agent_config(false);
    let view = spawn_target_agent_config(&cfg);
    assert!(
        !view.no_bootstrap,
        "no_bootstrap=false (default) must map to false"
    );
}
