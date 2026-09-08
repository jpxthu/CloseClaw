use std::path::PathBuf;

use async_trait::async_trait;
use closeclaw_common::{BootstrapMode, SessionRole};
use closeclaw_session::bootstrap::loader::{bootstrap_file_list, load_bootstrap_files};

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};

/// Provider that contributes bootstrap file content (agent profile, workspace
/// rules, etc.) to the system prompt.
///
/// Bootstrap files are loaded from `bootstrap_dir` using `ctx.bootstrap_mode`.
///
/// MEMORY.md is excluded — it is handled separately by
/// [`MemoryFragmentProvider`](super::memory::MemoryFragmentProvider).
pub struct BootstrapFragmentProvider;

impl Default for BootstrapFragmentProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl BootstrapFragmentProvider {
    pub fn new() -> Self {
        Self
    }

    /// Resolve bootstrap mode from context.
    ///
    /// Sub sessions are forced to Minimal mode regardless of the configured
    /// `bootstrap_mode` — BOOTSTRAP.md is only loaded for Main sessions.
    fn resolve_mode(&self, ctx: &FragmentContext) -> BootstrapMode {
        if ctx.session_role == SessionRole::Sub {
            return BootstrapMode::Minimal;
        }
        ctx.bootstrap_mode
    }

    /// Resolve the directory to load bootstrap files from.
    fn resolve_bootstrap_dir(&self, ctx: &FragmentContext) -> PathBuf {
        PathBuf::from(&ctx.bootstrap_dir)
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

        // Traverse files in the doc-defined fixed order from
        // `bootstrap_file_list`. MEMORY.md is excluded from the list
        // (handled by MemoryFragmentProvider). Skip any missing files.
        let ordered_names = bootstrap_file_list(mode);
        let mut entries: Vec<(&str, &String)> = Vec::new();
        for name in ordered_names {
            if let Some(body) = files.get(name) {
                entries.push((name, body));
            }
        }

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
        // the resolved mode. This must use the same resolve_mode() as
        // generate() so cache dimensions match generation dimensions.
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
    use closeclaw_common::SessionRole;
    use std::fs;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_provider_name_and_priority() {
        let provider = BootstrapFragmentProvider::new();
        assert_eq!(provider.name(), "bootstrap");
        assert_eq!(provider.priority(), 1);
    }

    #[test]
    fn test_resolve_mode_from_context() {
        let provider = BootstrapFragmentProvider::new();

        // Main session: use ctx.bootstrap_mode directly.
        let ctx = FragmentContext {
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Full);

        // Sub session: always returns Minimal regardless of bootstrap_mode.
        let ctx = FragmentContext {
            bootstrap_mode: BootstrapMode::Full,
            session_role: SessionRole::Sub,
            agent_id: String::new(),
            ..FragmentContext::test_default()
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Minimal);
    }

    #[test]
    fn test_resolve_mode_returns_ctx_value() {
        let provider = BootstrapFragmentProvider::new();

        // Main + Minimal → returns Minimal
        let ctx = FragmentContext {
            agent_id: "test-agent".into(),
            bootstrap_mode: BootstrapMode::Minimal,
            bootstrap_dir: std::env::temp_dir().to_string_lossy().to_string(),
            ..FragmentContext::test_default()
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Minimal);

        // Sub + Minimal → returns Minimal (forced by role guard)
        let ctx = FragmentContext {
            agent_id: "unknown".into(),
            session_role: SessionRole::Sub,
            bootstrap_mode: BootstrapMode::Minimal,
            bootstrap_dir: std::env::temp_dir().to_string_lossy().to_string(),
            ..FragmentContext::test_default()
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Minimal);

        // Sub + Full → returns Minimal (forced by role guard)
        let ctx = FragmentContext {
            agent_id: "unknown".into(),
            session_role: SessionRole::Sub,
            bootstrap_mode: BootstrapMode::Full,
            bootstrap_dir: std::env::temp_dir().to_string_lossy().to_string(),
            ..FragmentContext::test_default()
        };
        assert_eq!(provider.resolve_mode(&ctx), BootstrapMode::Minimal);
    }

    #[tokio::test]
    async fn test_generate_empty_dir_returns_none_with_full_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
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
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
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
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
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
    async fn test_generate_multi_files_fixed_order_full_mode() {
        let tmp = tempfile::tempdir().unwrap();
        // Create all Full-mode files (6 files, MEMORY.md is not in bootstrap list)
        fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul content").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity content").unwrap();
        fs::write(tmp.path().join("USER.md"), "user content").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools content").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap content").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        // Full mode fixed order: AGENTS → SOUL → IDENTITY → USER → TOOLS → BOOTSTRAP
        // BOOTSTRAP.md must be last; SOUL.md must be second.
        let expected = concat!(
            "## AGENTS.md\nagents content\n\n",
            "## SOUL.md\nsoul content\n\n",
            "## IDENTITY.md\nidentity content\n\n",
            "## USER.md\nuser content\n\n",
            "## TOOLS.md\ntools content\n\n",
            "## BOOTSTRAP.md\nbootstrap content",
        );
        assert_eq!(fragment.content, expected);
    }

    #[tokio::test]
    async fn test_generate_multi_files_fixed_order_minimal_mode() {
        let tmp = tempfile::tempdir().unwrap();
        // Create all Minimal-mode files (5 files, no BOOTSTRAP.md)
        fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul content").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity content").unwrap();
        fs::write(tmp.path().join("USER.md"), "user content").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools content").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        // Minimal mode fixed order: AGENTS → SOUL → IDENTITY → USER → TOOLS
        let expected = concat!(
            "## AGENTS.md\nagents content\n\n",
            "## SOUL.md\nsoul content\n\n",
            "## IDENTITY.md\nidentity content\n\n",
            "## USER.md\nuser content\n\n",
            "## TOOLS.md\ntools content",
        );
        assert_eq!(fragment.content, expected);
    }

    #[tokio::test]
    async fn test_generate_partial_files_preserve_order() {
        let tmp = tempfile::tempdir().unwrap();
        // Only SOUL.md and TOOLS.md exist — missing files are skipped,
        // relative order of remaining files must match doc-defined order.
        fs::write(tmp.path().join("SOUL.md"), "soul content").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools content").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        // SOUL.md (index 1) must come before TOOLS.md (index 4)
        let soul_pos = fragment.content.find("## SOUL.md").unwrap();
        let tools_pos = fragment.content.find("## TOOLS.md").unwrap();
        assert!(
            soul_pos < tools_pos,
            "SOUL.md should appear before TOOLS.md"
        );
        assert_eq!(
            fragment.content,
            "## SOUL.md\nsoul content\n\n## TOOLS.md\ntools content"
        );
    }

    #[tokio::test]
    async fn test_generate_excludes_memory_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "memory content").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        // MEMORY.md is not in bootstrap_file_list(Full), so it is never loaded
        // into the bootstrap result map. The generate() loop only iterates
        // files from bootstrap_file_list, so MEMORY.md content is excluded.
        assert!(!fragment.content.contains("memory content"));
        assert!(fragment.content.contains("agents content"));
    }

    #[test]
    fn test_cache_key_works_with_valid_workdir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "content").unwrap();
        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
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
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
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
            bootstrap_dir: "/work/path".to_string(),
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
            bootstrap_dir: "/definitely/does/not/exist".to_string(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[test]
    fn test_cache_key_nonexistent_workdir_returns_none() {
        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: "/definitely/does/not/exist".to_string(),
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
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
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
            session_role: SessionRole::Main,
            bootstrap_mode: BootstrapMode::Minimal,
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            activated_skills: vec![],
            tool_registry: None,
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
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
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
            session_role: SessionRole::Main,
            bootstrap_mode: BootstrapMode::Full,
            bootstrap_dir: std::env::temp_dir().to_string_lossy().to_string(),
            activated_skills: vec![],
            tool_registry: None,
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
            session_role: SessionRole::Main,
            bootstrap_mode: BootstrapMode::Minimal,
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            activated_skills: vec![],
            tool_registry: None,
        };

        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.content.contains("minimal content"));
    }

    // ============================================================
    // Step 1.2: Cache independence & loading behavior tests
    // ============================================================

    /// Verify that modifying MEMORY.md does NOT invalidate the bootstrap
    /// cache key — MEMORY.md is excluded from bootstrap_file_list(Full).
    /// The memory provider's cache_key must change independently.
    #[test]
    fn test_cache_key_independence_from_memory_md() {
        let tmp = tempfile::tempdir().unwrap();
        // Create all 6 bootstrap files
        fs::write(tmp.path().join("AGENTS.md"), "agents").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity").unwrap();
        fs::write(tmp.path().join("USER.md"), "user").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap").unwrap();
        // Also create MEMORY.md — present in workspace but excluded from
        // bootstrap_file_list(Full)
        fs::write(tmp.path().join("MEMORY.md"), "original memory").unwrap();

        let boot_provider = BootstrapFragmentProvider::new();
        let mem_provider =
            closeclaw_memory::memory_fragment_provider::MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };

        let boot_key_before = boot_provider.cache_key(&ctx);
        let mem_key_before = mem_provider.cache_key(&ctx);
        assert!(boot_key_before.is_some(), "bootstrap key should exist");
        assert!(mem_key_before.is_some(), "memory key should exist");

        // Touch MEMORY.md to change its mtime. On ext4/NIFS with 1-second
        // granularity we need to ensure the file is actually updated at a
        // different second, so we write with distinct content.
        thread::sleep(Duration::from_millis(1100));
        fs::write(tmp.path().join("MEMORY.md"), "updated memory content").unwrap();

        let boot_key_after = boot_provider.cache_key(&ctx);
        let mem_key_after = mem_provider.cache_key(&ctx);

        // Bootstrap cache key must NOT change — MEMORY.md is not in the
        // bootstrap file list, so its mtime is not inspected.
        assert_eq!(
            boot_key_before, boot_key_after,
            "bootstrap cache_key must be stable when only MEMORY.md changes"
        );
        // Memory cache key MUST change — it reflects MEMORY.md mtime.
        assert_ne!(
            mem_key_before, mem_key_after,
            "memory cache_key must change when MEMORY.md is modified"
        );
    }

    /// Verify bootstrap_file_list(Full) returns exactly 6 items without
    /// MEMORY.md, in the document-defined fixed order.
    #[test]
    fn test_bootstrap_file_list_full_excludes_memory_md() {
        let list = bootstrap_file_list(BootstrapMode::Full);
        assert_eq!(
            list.len(),
            6,
            "Full mode must have exactly 6 bootstrap files"
        );
        assert_eq!(
            list,
            vec![
                "AGENTS.md",
                "SOUL.md",
                "IDENTITY.md",
                "USER.md",
                "TOOLS.md",
                "BOOTSTRAP.md",
            ],
            "order must match doc-defined sequence"
        );
        assert!(
            !list.contains(&"MEMORY.md"),
            "MEMORY.md must not be in bootstrap list"
        );
    }

    /// Full mode load_bootstrap_files() must not include MEMORY.md in the
    /// result even when the file exists in the workspace directory.
    #[test]
    fn test_load_bootstrap_files_full_excludes_memory_md() {
        let tmp = tempfile::tempdir().unwrap();
        // Create all 6 bootstrap files + MEMORY.md
        fs::write(tmp.path().join("AGENTS.md"), "a").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "s").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "i").unwrap();
        fs::write(tmp.path().join("USER.md"), "u").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "t").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "b").unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "m").unwrap();

        let result = load_bootstrap_files(tmp.path(), BootstrapMode::Full).unwrap();

        assert_eq!(
            result.len(),
            6,
            "Full mode load must return exactly 6 entries"
        );
        assert!(
            !result.contains_key("MEMORY.md"),
            "MEMORY.md must not appear in load result"
        );
    }

    /// When MEMORY.md does not exist, bootstrap cache_key behavior must
    /// be identical to when it is absent from the list — i.e. the key is
    /// determined solely by the 6 bootstrap files.
    #[test]
    fn test_cache_key_stable_when_memory_md_absent() {
        let tmp = tempfile::tempdir().unwrap();
        // Create only bootstrap files, no MEMORY.md
        fs::write(tmp.path().join("AGENTS.md"), "agents").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity").unwrap();
        fs::write(tmp.path().join("USER.md"), "user").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap").unwrap();

        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        let key_without = provider.cache_key(&ctx);
        assert!(key_without.is_some(), "key should exist without MEMORY.md");

        // Now create MEMORY.md — key must NOT change
        thread::sleep(Duration::from_millis(1100));
        fs::write(tmp.path().join("MEMORY.md"), "memory").unwrap();

        let key_with = provider.cache_key(&ctx);
        assert_eq!(
            key_without, key_with,
            "bootstrap cache_key must not change when MEMORY.md appears"
        );
    }

    // ============================================================
    // Step 1.2: Four-quadrant session_role × bootstrap_mode tests
    // ============================================================

    /// Main + Full → must include BOOTSTRAP.md and all required files.
    #[tokio::test]
    async fn test_main_full_includes_bootstrap_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity").unwrap();
        fs::write(tmp.path().join("USER.md"), "user").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap").unwrap();

        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Full,
            session_role: SessionRole::Main,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.content.contains("AGENTS.md"));
        assert!(fragment.content.contains("BOOTSTRAP.md"));
        assert!(fragment.content.contains("bootstrap"));
    }

    /// Main + Minimal → must NOT include BOOTSTRAP.md, but required
    /// files (AGENTS, SOUL, etc.) must be present.
    #[tokio::test]
    async fn test_main_minimal_excludes_bootstrap_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity").unwrap();
        fs::write(tmp.path().join("USER.md"), "user").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap").unwrap();

        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Minimal,
            session_role: SessionRole::Main,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.content.contains("AGENTS.md"));
        assert!(fragment.content.contains("SOUL.md"));
        assert!(!fragment.content.contains("BOOTSTRAP.md"));
    }

    /// Sub + Full → must NOT include BOOTSTRAP.md (role guard forces
    /// Minimal mode), but required files (AGENTS, SOUL, etc.) must be present.
    #[tokio::test]
    async fn test_sub_full_excludes_bootstrap_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity").unwrap();
        fs::write(tmp.path().join("USER.md"), "user").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap").unwrap();

        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Full,
            session_role: SessionRole::Sub,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.content.contains("AGENTS.md"));
        assert!(fragment.content.contains("SOUL.md"));
        assert!(!fragment.content.contains("BOOTSTRAP.md"));
    }

    /// Sub + Minimal → must NOT include BOOTSTRAP.md, required files
    /// present.
    #[tokio::test]
    async fn test_sub_minimal_excludes_bootstrap_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity").unwrap();
        fs::write(tmp.path().join("USER.md"), "user").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap").unwrap();

        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Minimal,
            session_role: SessionRole::Sub,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await.unwrap();
        assert!(fragment.content.contains("AGENTS.md"));
        assert!(!fragment.content.contains("BOOTSTRAP.md"));
    }

    /// Sub + Full → cache_key must match Main + Minimal (both resolve to
    /// Minimal mode), confirming cache dimension aligns with generation.
    #[test]
    fn test_cache_key_sub_full_matches_main_minimal() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents").unwrap();
        fs::write(tmp.path().join("SOUL.md"), "soul").unwrap();
        fs::write(tmp.path().join("IDENTITY.md"), "identity").unwrap();
        fs::write(tmp.path().join("USER.md"), "user").unwrap();
        fs::write(tmp.path().join("TOOLS.md"), "tools").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap").unwrap();

        let provider = BootstrapFragmentProvider::new();

        let sub_full_ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Full,
            session_role: SessionRole::Sub,
            ..FragmentContext::test_default()
        };
        let main_minimal_ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Minimal,
            session_role: SessionRole::Main,
            ..FragmentContext::test_default()
        };

        let key_sub_full = provider.cache_key(&sub_full_ctx);
        let key_main_minimal = provider.cache_key(&main_minimal_ctx);

        // Both resolve to Minimal → same file list → same cache key.
        assert_eq!(
            key_sub_full, key_main_minimal,
            "Sub+Full cache_key must equal Main+Minimal (both resolve to Minimal)"
        );
    }

    /// Sub role → cache_key must NOT include BOOTSTRAP.md mtime.
    #[test]
    fn test_cache_key_sub_role_excludes_bootstrap_md_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "agents").unwrap();
        fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap").unwrap();

        let provider = BootstrapFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_string_lossy().to_string(),
            bootstrap_mode: BootstrapMode::Full,
            session_role: SessionRole::Sub,
            ..FragmentContext::test_default()
        };
        let key = provider.cache_key(&ctx).unwrap();
        assert!(
            !key.contains("BOOTSTRAP.md"),
            "Sub role cache_key must not include BOOTSTRAP.md mtime"
        );
        assert!(key.contains("AGENTS.md"));
    }
}
