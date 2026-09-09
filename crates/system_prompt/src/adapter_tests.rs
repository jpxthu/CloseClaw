//! Tests for SystemPromptBuilderAdapter.

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::system_prompt::PromptOverrides;
use closeclaw_common::{BootstrapMode, PromptFragmentProvider, SessionRole, SystemPromptBuilder};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::adapter::SystemPromptBuilderAdapter;
use crate::fragment::{FragmentContext, PromptFragment, SectionType};
use crate::providers::bootstrap::BootstrapFragmentProvider;
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Mock provider for adapter tests
// ---------------------------------------------------------------------------

struct MockProvider {
    name: String,
    priority: u32,
    content: Option<String>,
}

impl MockProvider {
    fn with_content(name: &str, priority: u32, content: &str) -> Self {
        Self {
            name: name.to_string(),
            priority,
            content: Some(content.to_string()),
        }
    }

    #[allow(dead_code)]
    fn empty(name: &str, priority: u32) -> Self {
        Self {
            name: name.to_string(),
            priority,
            content: None,
        }
    }
}

#[async_trait]
impl PromptFragmentProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    async fn generate(&self, _ctx: &FragmentContext) -> Option<PromptFragment> {
        self.content.as_ref().map(|c| PromptFragment {
            section_title: format!("## {}", self.name),
            section_type: SectionType::Bootstrap,
            content: c.clone(),
        })
    }

    fn cache_key(&self, _ctx: &FragmentContext) -> Option<String> {
        None
    }
}

/// Helper to create a test adapter with a temporary workspace.
fn test_adapter(
    workspace: &std::path::Path,
    providers: Vec<Arc<dyn PromptFragmentProvider>>,
) -> SystemPromptBuilderAdapter {
    let agent_registry = Arc::new(RwLock::new(AgentRegistry::new()));
    SystemPromptBuilderAdapter::new(agent_registry, workspace.to_path_buf(), providers)
}

/// Helper to create a test adapter with a pre-populated agent registry.
async fn test_adapter_with_agent(
    workspace: &std::path::Path,
    agent_id: &str,
    bootstrap_mode: BootstrapMode,
    providers: Vec<Arc<dyn PromptFragmentProvider>>,
) -> SystemPromptBuilderAdapter {
    use closeclaw_agent::config::AgentConfig;
    use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};

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
    SystemPromptBuilderAdapter::new(agent_registry, workspace.to_path_buf(), providers)
}

/// Build a provider list with the real BootstrapFragmentProvider.
/// Uses Arc so it can be shared across multiple build calls.
fn bootstrap_providers() -> Vec<Arc<dyn PromptFragmentProvider>> {
    vec![Arc::new(BootstrapFragmentProvider::new())]
}

#[tokio::test]
async fn test_build_prompt_returns_non_empty_string() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    // Create a minimal workspace with a bootstrap file.
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("BOOTSTRAP.md"), "bootstrap file content").unwrap();

    let adapter = test_adapter(tmp.path(), bootstrap_providers());
    let result = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
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

    let adapter = test_adapter(tmp.path(), bootstrap_providers());
    let result = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
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

    let adapter = test_adapter(tmp.path(), bootstrap_providers());
    // Build once to populate the cache.
    let result_before = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
        .await;
    assert!(!result_before.is_empty());

    // Verify cache is populated by building again (should be cached).
    let result_cached = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
        .await;
    assert_eq!(result_before, result_cached);

    // Invalidate the cache.
    adapter.invalidate_cache().await;

    // After invalidation, build should regenerate (still works correctly).
    let result_after = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
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
    std::fs::write(ws.join("BOOTSTRAP.md"), "original content").unwrap();

    let adapter = test_adapter(tmp.path(), bootstrap_providers());
    let overrides = PromptOverrides {
        override_prompt: Some("REPLACED".to_string()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let result = adapter
        .build_prompt(
            "session-1",
            agent_id,
            Some(&overrides),
            None,
            SessionRole::Main,
        )
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
    std::fs::write(ws.join("BOOTSTRAP.md"), "base content").unwrap();

    let adapter = test_adapter(tmp.path(), bootstrap_providers());
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: Some("agent extra".to_string()),
        custom_prompt: None,
    };
    let result = adapter
        .build_prompt(
            "session-1",
            agent_id,
            Some(&overrides),
            None,
            SessionRole::Main,
        )
        .await;
    // BootstrapFragmentProvider reads BOOTSTRAP.md from the workspace dir.
    assert!(result.contains("base content"));
    assert!(result.contains("agent extra"));
}

#[tokio::test]
async fn test_prompt_overrides_priority_override_gt_agent_gt_custom() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let adapter = test_adapter(tmp.path(), bootstrap_providers());
    let overrides = PromptOverrides {
        override_prompt: Some("OVERRIDE".to_string()),
        agent_prompt: Some("AGENT".to_string()),
        custom_prompt: Some("CUSTOM".to_string()),
    };
    let result = adapter
        .build_prompt(
            "session-1",
            agent_id,
            Some(&overrides),
            None,
            SessionRole::Main,
        )
        .await;
    // override_prompt replaces everything; agent_prompt and custom_prompt are ignored.
    assert_eq!(result, "OVERRIDE");
}

#[tokio::test]
async fn test_workspace_not_exists_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    // No workspace directory created — adapter should degrade gracefully.
    let adapter = test_adapter(tmp.path(), bootstrap_providers());
    let result = adapter
        .build_prompt(
            "session-1",
            "nonexistent-agent",
            None,
            None,
            SessionRole::Main,
        )
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
    let adapter = test_adapter_with_agent(
        tmp.path(),
        agent_id,
        BootstrapMode::Minimal,
        bootstrap_providers(),
    )
    .await;

    // Override with Full mode — should include BOOTSTRAP.md content.
    let result = adapter
        .build_prompt(
            "session-1",
            agent_id,
            None,
            Some(BootstrapMode::Full),
            SessionRole::Main,
        )
        .await;
    // The bootstrap provider reads BOOTSTRAP.md from disk in Full mode.
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
    let adapter = test_adapter_with_agent(
        tmp.path(),
        agent_id,
        BootstrapMode::Minimal,
        bootstrap_providers(),
    )
    .await;

    let result = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
        .await;
    assert!(
        !result.contains("bootstrap only in full"),
        "Minimal mode should exclude BOOTSTRAP.md, got: {}",
        result
    );
}

/// Adapter with multiple providers respects priority ordering.
#[tokio::test]
async fn test_adapter_multiple_providers_priority() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let providers: Vec<Arc<dyn PromptFragmentProvider>> = vec![
        Arc::new(MockProvider::with_content("memory", 4, "memory content")),
        Arc::new(MockProvider::with_content(
            "bootstrap",
            1,
            "bootstrap content",
        )),
        Arc::new(MockProvider::with_content("tools", 2, "tools content")),
    ];

    let adapter = test_adapter(tmp.path(), providers);
    let result = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
        .await;

    // All three providers should contribute.
    assert!(result.contains("bootstrap content"));
    assert!(result.contains("tools content"));
    assert!(result.contains("memory content"));

    // Priority ordering: bootstrap (1) < tools (2) < memory (4).
    let boot_pos = result.find("bootstrap content").unwrap();
    let tool_pos = result.find("tools content").unwrap();
    let mem_pos = result.find("memory content").unwrap();
    assert!(boot_pos < tool_pos);
    assert!(tool_pos < mem_pos);
}

// ------------------------------------------------------------------
// Dimension: build_prompt_with_activated — adapter passes activated
// skills through to the provider pipeline
// ------------------------------------------------------------------

/// Mock provider that records the activated_skills it received via
/// FragmentContext, for integration testing.
struct ActivationRecordingProvider {
    recorded: std::sync::Arc<tokio::sync::Mutex<Vec<String>>>,
}

#[async_trait]
impl PromptFragmentProvider for ActivationRecordingProvider {
    fn name(&self) -> &str {
        "activation_recorder"
    }

    fn priority(&self) -> u32 {
        3 // same as SkillsFragmentProvider
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        let mut guard = self.recorded.lock().await;
        *guard = ctx.activated_skills.clone();
        if ctx.activated_skills.is_empty() {
            return Some(PromptFragment {
                section_title: "## Skills".to_string(),
                section_type: SectionType::Skills,
                content: "- **base_skill**: base".to_string(),
            });
        }
        let skills_text: Vec<String> = ctx
            .activated_skills
            .iter()
            .map(|s| format!("- **{}**: activated", s))
            .collect();
        let mut content = "- **base_skill**: base".to_string();
        for s in &skills_text {
            content.push('\n');
            content.push_str(s);
        }
        Some(PromptFragment {
            section_title: "## Skills".to_string(),
            section_type: SectionType::Skills,
            content,
        })
    }

    fn cache_key(&self, _ctx: &FragmentContext) -> Option<String> {
        None
    }
}

#[tokio::test]
async fn test_build_prompt_with_activated_passes_skills_to_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let recorded = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let provider = ActivationRecordingProvider {
        recorded: recorded.clone(),
    };

    let adapter = test_adapter(tmp.path(), vec![Arc::new(provider)]);

    // Build with activated skills
    let activated = vec!["cond_skill_a".to_string(), "cond_skill_b".to_string()];
    let result = adapter
        .build_prompt_with_activated(
            "session-1",
            agent_id,
            None,
            None,
            activated.clone(),
            SessionRole::Main,
        )
        .await;

    // Verify the provider received the activated skills
    let guard = recorded.lock().await;
    assert_eq!(*guard, activated);
    drop(guard);

    // Verify the output contains the activated skills
    assert!(result.contains("cond_skill_a"));
    assert!(result.contains("cond_skill_b"));
    assert!(result.contains("base_skill"));
}

#[tokio::test]
async fn test_build_prompt_without_activated_empty_ctx() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let recorded = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let provider = ActivationRecordingProvider {
        recorded: recorded.clone(),
    };

    let adapter = test_adapter(tmp.path(), vec![Arc::new(provider)]);

    // Build without activated skills (empty vec)
    let result = adapter
        .build_prompt_with_activated("session-1", agent_id, None, None, vec![], SessionRole::Main)
        .await;

    // Provider should receive empty activated_skills
    let guard = recorded.lock().await;
    assert!(guard.is_empty());
    drop(guard);

    // Output should only contain base skill
    assert!(result.contains("base_skill"));
    assert!(!result.contains("cond_skill"));
}

#[tokio::test]
async fn test_build_prompt_vs_with_activated_different_output() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let recorded = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let provider = ActivationRecordingProvider {
        recorded: recorded.clone(),
    };

    let adapter = test_adapter(tmp.path(), vec![Arc::new(provider)]);

    // Build without activated skills
    let result_base = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
        .await;

    // Build with activated skills
    let result_activated = adapter
        .build_prompt_with_activated(
            "session-1",
            agent_id,
            None,
            None,
            vec!["cond_skill".to_string()],
            SessionRole::Main,
        )
        .await;

    // Results should differ because activated skills change the output
    assert_ne!(result_base, result_activated);
    assert!(result_activated.contains("cond_skill"));
}

// ------------------------------------------------------------------
// Dimension: pass-through correctness — adapter propagates
// session_role into FragmentContext without mutation
// ------------------------------------------------------------------

struct RoleRecordingProvider {
    recorded: std::sync::Arc<tokio::sync::Mutex<Vec<SessionRole>>>,
}

#[async_trait]
impl PromptFragmentProvider for RoleRecordingProvider {
    fn name(&self) -> &str {
        "role_recorder"
    }

    fn priority(&self) -> u32 {
        5
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        let mut guard = self.recorded.lock().await;
        guard.push(ctx.session_role);
        Some(PromptFragment {
            section_title: "## Role".to_string(),
            section_type: SectionType::Bootstrap,
            content: format!("role={:?}", ctx.session_role),
        })
    }

    fn cache_key(&self, _ctx: &FragmentContext) -> Option<String> {
        None
    }
}

/// Adapter passes Sub role from caller into FragmentContext as-is.
#[tokio::test]
async fn test_adapter_passes_sub_role_to_fragment_context() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let recorded = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<SessionRole>::new()));
    let provider = RoleRecordingProvider {
        recorded: recorded.clone(),
    };
    let adapter = test_adapter(tmp.path(), vec![Arc::new(provider)]);

    let _result = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Sub)
        .await;

    let roles = recorded.lock().await;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0], SessionRole::Sub);
}

/// Adapter passes Main role from caller into FragmentContext as-is.
#[tokio::test]
async fn test_adapter_passes_main_role_to_fragment_context() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let recorded = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<SessionRole>::new()));
    let provider = RoleRecordingProvider {
        recorded: recorded.clone(),
    };
    let adapter = test_adapter(tmp.path(), vec![Arc::new(provider)]);

    let _result = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
        .await;

    let roles = recorded.lock().await;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0], SessionRole::Main);
}

// ------------------------------------------------------------------
// Dimension: tool_registry propagation — build_prompt_with_params
// passes the ToolRegistry reference through WorkspaceBuildConfig
// into FragmentContext.
// ------------------------------------------------------------------

struct ToolRegistryRecordingProvider {
    recorded: std::sync::Arc<
        tokio::sync::Mutex<
            Vec<Option<Arc<dyn closeclaw_common::tool_registry::ToolRegistryQuery>>>,
        >,
    >,
}

#[async_trait]
impl PromptFragmentProvider for ToolRegistryRecordingProvider {
    fn name(&self) -> &str {
        "registry_recorder"
    }

    fn priority(&self) -> u32 {
        6
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        let mut guard = self.recorded.lock().await;
        guard.push(ctx.tool_registry.clone());
        Some(PromptFragment {
            section_title: "## Registry".to_string(),
            section_type: SectionType::Tools,
            content: "registry test".to_string(),
        })
    }

    fn cache_key(&self, _ctx: &FragmentContext) -> Option<String> {
        None
    }
}

/// build_prompt_with_params propagates tool_registry from
/// InjectionParams through WorkspaceBuildConfig into FragmentContext.
#[tokio::test]
async fn test_build_prompt_with_params_propagates_tool_registry() {
    use closeclaw_common::injection_params::InjectionParams;
    use closeclaw_common::SessionRole;

    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let recorded = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<
        Option<Arc<dyn closeclaw_common::tool_registry::ToolRegistryQuery>>,
    >::new()));
    let provider = ToolRegistryRecordingProvider {
        recorded: recorded.clone(),
    };
    let adapter = test_adapter(tmp.path(), vec![Arc::new(provider)]);

    // Create a fake ToolRegistryQuery implementation.
    let fake_registry: Arc<dyn closeclaw_common::tool_registry::ToolRegistryQuery> =
        Arc::new(closeclaw_tools::ToolRegistry::new());

    let params = InjectionParams {
        session_id: "session-1".to_string(),
        agent_id: agent_id.to_string(),
        overrides: None,
        bootstrap_mode_override: None,
        activated_skills: vec![],
        session_role: SessionRole::Main,
        tool_registry: Some(fake_registry.clone()),
    };

    let _result = adapter.build_prompt_with_params(&params).await;

    let registries = recorded.lock().await;
    assert_eq!(registries.len(), 1, "provider should be called once");
    assert!(
        registries[0].is_some(),
        "tool_registry should be propagated as Some"
    );
}

/// build_prompt (without params) passes tool_registry as None.
#[tokio::test]
async fn test_build_prompt_passes_tool_registry_none() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "test-agent";
    let ws = tmp.path().join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();

    let recorded = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<
        Option<Arc<dyn closeclaw_common::tool_registry::ToolRegistryQuery>>,
    >::new()));
    let provider = ToolRegistryRecordingProvider {
        recorded: recorded.clone(),
    };
    let adapter = test_adapter(tmp.path(), vec![Arc::new(provider)]);

    let _result = adapter
        .build_prompt("session-1", agent_id, None, None, SessionRole::Main)
        .await;

    let registries = recorded.lock().await;
    assert_eq!(registries.len(), 1, "provider should be called once");
    assert!(
        registries[0].is_none(),
        "tool_registry should be None when not provided via params"
    );
}
