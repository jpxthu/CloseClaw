//! Tests for SystemPromptBuilderAdapter.

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::system_prompt::PromptOverrides;
use closeclaw_common::{BootstrapMode, SystemPromptBuilder};
use closeclaw_tools::ToolRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::adapter::SystemPromptBuilderAdapter;

/// Helper to create a test adapter with a temporary workspace.
fn test_adapter(workspace: &std::path::Path) -> SystemPromptBuilderAdapter {
    let tool_registry = Arc::new(ToolRegistry::new());
    let agent_registry = Arc::new(RwLock::new(AgentRegistry::new()));
    SystemPromptBuilderAdapter::new(tool_registry, agent_registry, workspace.to_path_buf())
}

/// Helper to create a test adapter with a pre-populated agent registry.
async fn test_adapter_with_agent(
    workspace: &std::path::Path,
    agent_id: &str,
    bootstrap_mode: BootstrapMode,
) -> SystemPromptBuilderAdapter {
    use closeclaw_agent::config::AgentConfig;
    use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};

    let tool_registry = Arc::new(ToolRegistry::new());
    let agent_registry = Arc::new(RwLock::new(AgentRegistry::new()));
    // Create and populate the agent config.
    let agent_config = AgentConfig {
        id: agent_id.to_string(),
        ..Default::default()
    };
    let resolved =
        ResolvedAgentConfig::from_single(agent_config, ConfigSource::User, "<test>", None).unwrap();
    // Override bootstrap_mode after resolution.
    let mut resolved = resolved;
    resolved.bootstrap_mode = bootstrap_mode;
    {
        let reg = agent_registry.write().await;
        reg.populate(vec![resolved]);
    }
    SystemPromptBuilderAdapter::new(tool_registry, agent_registry, workspace.to_path_buf())
}

#[tokio::test]
async fn test_build_prompt_returns_non_empty_string() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    // Create a minimal workspace with a bootstrap file.
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("AGENTS.md"), "agents content").unwrap();

    let adapter = test_adapter(tmp.path());
    let result = adapter
        .build_prompt("session-1", agent_id, None, None)
        .await;
    assert!(!result.is_empty());
}

#[tokio::test]
async fn test_build_prompt_includes_bootstrap_content() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("BOOTSTRAP.md"), "bootstrap content here").unwrap();

    let adapter = test_adapter(tmp.path());
    let result = adapter
        .build_prompt("session-1", agent_id, None, None)
        .await;
    assert!(
        result.contains("bootstrap content here"),
        "expected bootstrap content in prompt, got: {}",
        result
    );
}

#[tokio::test]
async fn test_invalidate_cache_clears_sections() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("AGENTS.md"), "agents content").unwrap();

    let adapter = test_adapter(tmp.path());
    // Build once to populate the cache.
    let result_before = adapter
        .build_prompt("session-1", agent_id, None, None)
        .await;
    assert!(!result_before.is_empty());

    // Verify cache is populated by building again (should be cached).
    let result_cached = adapter
        .build_prompt("session-1", agent_id, None, None)
        .await;
    assert_eq!(result_before, result_cached);

    // Invalidate the cache.
    adapter.invalidate_cache().await;

    // After invalidation, build should regenerate (still works correctly).
    let result_after = adapter
        .build_prompt("session-1", agent_id, None, None)
        .await;
    assert_eq!(
        result_before, result_after,
        "content should be same after invalidation and rebuild"
    );
}

#[tokio::test]
async fn test_prompt_overrides_override_replaces_static() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("AGENTS.md"), "original content").unwrap();

    let adapter = test_adapter(tmp.path());
    let overrides = PromptOverrides {
        override_prompt: Some("REPLACED".to_string()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let result = adapter
        .build_prompt("session-1", agent_id, Some(&overrides), None)
        .await;
    assert_eq!(result, "REPLACED");
    assert!(!result.contains("original content"));
}

#[tokio::test]
async fn test_prompt_overrides_agent_prompt_appends() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("AGENTS.md"), "base content").unwrap();

    let adapter = test_adapter(tmp.path());
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: Some("agent extra".to_string()),
        custom_prompt: None,
    };
    let result = adapter
        .build_prompt("session-1", agent_id, Some(&overrides), None)
        .await;
    assert!(result.contains("base content"));
    assert!(result.contains("agent extra"));
}

#[tokio::test]
async fn test_prompt_overrides_priority_override_gt_agent_gt_custom() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let adapter = test_adapter(tmp.path());
    let overrides = PromptOverrides {
        override_prompt: Some("OVERRIDE".to_string()),
        agent_prompt: Some("AGENT".to_string()),
        custom_prompt: Some("CUSTOM".to_string()),
    };
    let result = adapter
        .build_prompt("session-1", agent_id, Some(&overrides), None)
        .await;
    // override_prompt replaces everything; agent_prompt and custom_prompt are ignored.
    assert_eq!(result, "OVERRIDE");
}

#[tokio::test]
async fn test_workspace_not_exists_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    // No workspace directory created — adapter should degrade gracefully.
    let adapter = test_adapter(tmp.path());
    let result = adapter
        .build_prompt("session-1", "nonexistent-agent", None, None)
        .await;
    // Should return DEFAULT_PROMPT since no workspace exists.
    assert!(!result.is_empty());
}

#[tokio::test]
async fn test_bootstrap_mode_override_takes_precedence() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    // BOOTSTRAP.md is only loaded in Full mode.
    std::fs::write(ws.join("BOOTSTRAP.md"), "bootstrap only in full").unwrap();

    // Agent configured with Minimal mode.
    let adapter = test_adapter_with_agent(tmp.path(), agent_id, BootstrapMode::Minimal).await;

    // Override with Full mode — should include BOOTSTRAP.md content.
    let result = adapter
        .build_prompt("session-1", agent_id, None, Some(BootstrapMode::Full))
        .await;
    assert!(
        result.contains("bootstrap only in full"),
        "override should force Full mode, got: {}",
        result
    );
}

#[tokio::test]
async fn test_bootstrap_mode_from_registry_when_no_override() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("BOOTSTRAP.md"), "bootstrap only in full").unwrap();

    // Agent configured with Minimal mode — should NOT load BOOTSTRAP.md.
    let adapter = test_adapter_with_agent(tmp.path(), agent_id, BootstrapMode::Minimal).await;

    let result = adapter
        .build_prompt("session-1", agent_id, None, None)
        .await;
    assert!(
        !result.contains("bootstrap only in full"),
        "Minimal mode should exclude BOOTSTRAP.md, got: {}",
        result
    );
}
