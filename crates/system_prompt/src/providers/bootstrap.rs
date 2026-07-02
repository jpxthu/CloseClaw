use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::BootstrapMode;
use closeclaw_session::bootstrap::loader::load_bootstrap_files;

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};

/// Provider that contributes bootstrap file content (agent profile, workspace
/// rules, etc.) to the system prompt.
///
/// Bootstrap files are loaded from the agent's working directory. When
/// `FragmentContext::bootstrap_mode` is `None`, falls back to
/// [`AgentRegistry`] lookup.
///
/// MEMORY.md is excluded — it is handled separately by
/// [`MemoryFragmentProvider`](super::memory::MemoryFragmentProvider).
pub struct BootstrapFragmentProvider {
    agent_registry: Arc<AgentRegistry>,
}

impl BootstrapFragmentProvider {
    pub fn new(agent_registry: Arc<AgentRegistry>) -> Self {
        Self { agent_registry }
    }

    /// Resolve bootstrap mode from context, falling back to AgentRegistry.
    fn resolve_mode(&self, ctx: &FragmentContext) -> Option<BootstrapMode> {
        ctx.bootstrap_mode.or_else(|| {
            ctx.agent_id
                .as_deref()
                .and_then(|id| self.agent_registry.query_bootstrap_mode(id))
        })
    }
}

#[async_trait]
impl PromptFragmentProvider for BootstrapFragmentProvider {
    fn name(&self) -> &str {
        "bootstrap"
    }

    fn priority(&self) -> u32 {
        1
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        let workdir = ctx.workdir.as_ref()?;

        let mode = self.resolve_mode(ctx)?;
        let files = load_bootstrap_files(workdir, mode).ok()?;

        // Filter out MEMORY.md (handled by MemoryFragmentProvider) and sort by
        // filename for deterministic output.
        let mut entries: Vec<_> = files
            .into_iter()
            .filter(|(name, _)| name != "MEMORY.md")
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        if entries.is_empty() {
            return None;
        }

        // Each bootstrap file gets its own `## filename` header.
        let content: String = entries
            .iter()
            .map(|(name, body)| format!("## {}\n{}", name, body))
            .collect::<Vec<_>>()
            .join("\n\n");

        Some(PromptFragment {
            title: String::new(),
            section_type: SectionType::Bootstrap,
            content,
        })
    }

    fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        let workdir = ctx.workdir.as_ref()?;
        let mode = self.resolve_mode(ctx)?;

        // Build a cache key from file modification times without loading
        // the full file contents — just iterate the known file list for
        // this bootstrap mode.
        let file_names = closeclaw_session::bootstrap::loader::bootstrap_file_list(mode);
        let mut key_parts: Vec<String> = Vec::new();

        for name in file_names {
            let path = workdir.join(name);
            match std::fs::metadata(&path) {
                Ok(meta) => {
                    key_parts.push(format!(
                        "{}:{:?}",
                        name,
                        meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    ));
                }
                Err(_) => continue,
            }
        }

        if key_parts.is_empty() {
            return None;
        }

        Some(format!("bootstrap:{}", key_parts.join("|")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_provider_name_and_priority() {
        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);
        assert_eq!(provider.name(), "bootstrap");
        assert_eq!(provider.priority(), 1);
    }

    #[test]
    fn test_resolve_mode_from_context() {
        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);

        // When context has bootstrap_mode, use it directly.
        let ctx = FragmentContext {
            bootstrap_mode: Some(BootstrapMode::Full),
            ..Default::default()
        };
        assert_eq!(provider.resolve_mode(&ctx), Some(BootstrapMode::Full));

        // When context has no bootstrap_mode and no agent_id, return None.
        let ctx = FragmentContext::default();
        assert_eq!(provider.resolve_mode(&ctx), None);
    }

    #[test]
    fn test_resolve_mode_fallback_to_registry() {
        use closeclaw_agent::registry::AgentRegistry;
        use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};

        let reg = Arc::new(AgentRegistry::new());
        reg.populate(vec![ResolvedAgentConfig {
            id: "test-agent".into(),
            name: "test-agent".into(),
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: BootstrapMode::Minimal,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: Default::default(),
            memory: None,
            source: ConfigSource::User,
        }]);

        let provider = BootstrapFragmentProvider::new(reg);

        // Context without bootstrap_mode but with agent_id → fallback to registry.
        let ctx = FragmentContext {
            agent_id: Some("test-agent".into()),
            bootstrap_mode: None,
            workdir: None,
            agent_dir: None,
        };
        assert_eq!(provider.resolve_mode(&ctx), Some(BootstrapMode::Minimal));

        // Unknown agent_id → None.
        let ctx = FragmentContext {
            agent_id: Some("unknown".into()),
            bootstrap_mode: None,
            workdir: None,
            agent_dir: None,
        };
        assert_eq!(provider.resolve_mode(&ctx), None);
    }

    #[tokio::test]
    async fn test_generate_no_workdir_returns_none() {
        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);

        let ctx = FragmentContext::default();
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_empty_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);

        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            bootstrap_mode: Some(BootstrapMode::Minimal),
            ..Default::default()
        };
        // Minimal mode expects AGENTS.md etc. — none exist in temp dir.
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "# Agent Config\nHello").unwrap();

        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);

        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            bootstrap_mode: Some(BootstrapMode::Minimal),
            ..Default::default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.title.is_empty());
        assert_eq!(fragment.section_type, SectionType::Bootstrap);
        assert!(fragment.content.starts_with("## AGENTS.md\n"));
        assert!(fragment.content.contains("# Agent Config\nHello"));
    }

    #[tokio::test]
    async fn test_generate_multi_files_sorted_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul content").unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity content").unwrap();

        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);

        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            bootstrap_mode: Some(BootstrapMode::Minimal),
            ..Default::default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        // Files sorted by name: AGENTS.md, IDENTITY.md, SOUL.md
        // Each file gets its own ## header.
        assert!(fragment.content.starts_with("## AGENTS.md\nagents content"));
        assert!(fragment
            .content
            .contains("## IDENTITY.md\nidentity content"));
        assert!(fragment.content.contains("## SOUL.md\nsoul content"));
        assert_eq!(fragment.content.matches("\n\n").count(), 2);
    }

    #[tokio::test]
    async fn test_generate_excludes_memory_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "memory content").unwrap();

        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);

        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            bootstrap_mode: Some(BootstrapMode::Full),
            ..Default::default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(!fragment.content.contains("memory content"));
        assert!(fragment.content.contains("agents content"));
    }

    #[test]
    fn test_cache_key_none_when_no_workdir() {
        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);
        let ctx = FragmentContext::default();
        assert!(provider.cache_key(&ctx).is_none());
    }

    #[test]
    fn test_cache_key_varies_with_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "content").unwrap();

        let reg = Arc::new(AgentRegistry::new());
        let provider = BootstrapFragmentProvider::new(reg);

        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            bootstrap_mode: Some(BootstrapMode::Minimal),
            ..Default::default()
        };

        let key1 = provider.cache_key(&ctx);
        assert!(key1.is_some());
        // Same content → same key
        let key2 = provider.cache_key(&ctx);
        assert_eq!(key1, key2);
    }
}
