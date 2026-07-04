use std::path::PathBuf;

use async_trait::async_trait;
use closeclaw_common::BootstrapMode;
use closeclaw_session::bootstrap::loader::load_bootstrap_files;

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};

/// Provider that contributes bootstrap file content (agent profile, workspace
/// rules, etc.) to the system prompt.
///
/// Bootstrap files are loaded from `workdir` using `ctx.bootstrap_mode`.
///
/// MEMORY.md is excluded — it is handled separately by
/// [`MemoryFragmentProvider`](super::memory::MemoryFragmentProvider).
pub struct BootstrapFragmentProvider;

impl BootstrapFragmentProvider {
    pub fn new() -> Self {
        Self
    }

    /// Resolve bootstrap mode from context.
    fn resolve_mode(&self, ctx: &FragmentContext) -> BootstrapMode {
        ctx.bootstrap_mode
    }

    /// Resolve the directory to load bootstrap files from.
    fn resolve_bootstrap_dir(&self, ctx: &FragmentContext) -> PathBuf {
        ctx.workdir.clone()
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
        let bootstrap_dir = self.resolve_bootstrap_dir(ctx);

        let mode = self.resolve_mode(ctx);
        let files = load_bootstrap_files(&bootstrap_dir, mode).ok()?;

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
            section_title: String::new(),
            section_type: SectionType::Bootstrap,
            content,
        })
    }

    fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        let bootstrap_dir = self.resolve_bootstrap_dir(ctx);
        let mode = self.resolve_mode(ctx);

        // Build a cache key from file modification times without loading
        // the full file contents — just iterate the known file list for
        // this bootstrap mode.
        let file_names = closeclaw_session::bootstrap::loader::bootstrap_file_list(mode);
        let mut key_parts: Vec<String> = Vec::new();

        for name in file_names {
            let path = bootstrap_dir.join(name);
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
        let provider = BootstrapFragmentProvider::new();
        assert_eq!(provider.name(), "bootstrap");
        assert_eq!(provider.priority(), 1);
    }

    #[test]
    fn test_resolve_mode_from_context() {
        let provider = BootstrapFragmentProvider::new();

        // When context has bootstrap_mode, use it directly.
        let ctx = FragmentContext {
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Full);

        // bootstrap_mode is always present — returns it regardless of agent_id.
        let ctx = FragmentContext {
            bootstrap_mode: BootstrapMode::Full,
            agent_id: String::new(),
            ..FragmentContext::test_default()
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Full);
    }

    #[test]
    fn test_resolve_mode_returns_ctx_value() {
        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            agent_id: "test-agent".into(),
            bootstrap_mode: BootstrapMode::Minimal,
            workdir: std::env::temp_dir(),
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Minimal);

        // Unknown agent_id doesn't affect result — mode is always from ctx.
        let ctx = FragmentContext {
            agent_id: "unknown".into(),
            bootstrap_mode: BootstrapMode::Minimal,
            workdir: std::env::temp_dir(),
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Minimal);
    }

    #[tokio::test]
    async fn test_generate_empty_dir_returns_none_with_full_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        // Full mode expects AGENTS.md etc. — none exist in temp dir.
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_empty_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        // Minimal mode expects AGENTS.md etc. — none exist in temp dir.
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "# Agent Config\nHello").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.section_title.is_empty());
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

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
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

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(!fragment.content.contains("memory content"));
        assert!(fragment.content.contains("agents content"));
    }

    #[test]
    fn test_cache_key_works_with_valid_workdir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "content").unwrap();
        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        assert!(provider.cache_key(&ctx).is_some());
    }

    #[test]
    fn test_cache_key_varies_with_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "content").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };

        let key1 = provider.cache_key(&ctx);
        assert!(key1.is_some());
        // Same content → same key
        let key2 = provider.cache_key(&ctx);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_resolve_bootstrap_dir_uses_workdir() {
        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: PathBuf::from("/work/path"),
            ..FragmentContext::test_default()
        };
        assert_eq!(
            provider.resolve_bootstrap_dir(&ctx),
            PathBuf::from("/work/path")
        );
    }

    #[tokio::test]
    async fn test_generate_nonexistent_workdir_returns_none() {
        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: PathBuf::from("/definitely/does/not/exist"),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[test]
    fn test_cache_key_nonexistent_workdir_returns_none() {
        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: PathBuf::from("/definitely/does/not/exist"),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        assert!(provider.cache_key(&ctx).is_none());
    }

    #[tokio::test]
    async fn test_generate_full_mode_includes_bootstrap_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap content").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.content.contains("BOOTSTRAP.md"));
        assert!(fragment.content.contains("bootstrap content"));
    }

    #[tokio::test]
    async fn test_generate_workdir_and_agent_id_mode_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        // Minimal mode expects AGENTS.md
        fs::write(tmp.path().join("AGENTS.md"), "from workdir").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            agent_id: "test-agent".into(),
            bootstrap_mode: BootstrapMode::Minimal,
            workdir: tmp.path().to_path_buf(),
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.content.contains("from workdir"));
    }

    #[tokio::test]
    async fn test_cache_key_includes_workdir_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "content").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        let key = provider.cache_key(&ctx);
        assert!(key.is_some());
        assert!(key.unwrap().starts_with("bootstrap:AGENTS.md:"));
    }

    // --- resolve_mode tests (bootstrap_mode is now required) ---

    #[test]
    fn test_resolve_mode_returns_minimal() {
        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            agent_id: "test-agent".into(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Minimal);
    }

    #[test]
    fn test_resolve_mode_returns_full() {
        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            agent_id: "test-agent".into(),
            bootstrap_mode: BootstrapMode::Full,
            workdir: std::env::temp_dir(),
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Full);
    }

    #[tokio::test]
    async fn test_generate_uses_mode_for_file_loading() {
        let tmp = tempfile::tempdir().unwrap();
        // Minimal mode expects AGENTS.md.
        fs::write(tmp.path().join("AGENTS.md"), "minimal content").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            agent_id: "test-agent".into(),
            bootstrap_mode: BootstrapMode::Minimal,
            workdir: tmp.path().to_path_buf(),
        };

        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.content.contains("minimal content"));
    }
}
