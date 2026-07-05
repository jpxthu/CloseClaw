//! Provider for the Memory section of the system prompt.
//!
//! Reads `MEMORY.md` from the agent's working directory and wraps the
//! content as a [`PromptFragment`].

use std::path::PathBuf;

use async_trait::async_trait;

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};
use crate::sections::load_cached_file_section;

/// Provider that contributes the long-term memory (`MEMORY.md`) to the
/// system prompt. The file is read from the agent's working directory
/// (or a configured path).
///
/// When the file does not exist or is empty,
/// [`generate`](Self::generate) returns `None`.
pub struct MemoryFragmentProvider {
    /// Configured path to the MEMORY.md file.
    /// When `None`, falls back to `workdir.join("MEMORY.md")`.
    memory_md_path: Option<PathBuf>,
}

impl MemoryFragmentProvider {
    /// Create a new provider with no custom path (uses `workdir/MEMORY.md`).
    pub fn new() -> Self {
        Self {
            memory_md_path: None,
        }
    }

    /// Create a new provider with a custom MEMORY.md path.
    ///
    /// The path can be absolute or relative. When relative, it is resolved
    /// against the agent's working directory.
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self {
            memory_md_path: Some(path.into()),
        }
    }

    /// Resolve the MEMORY.md path for a given context.
    fn resolve_path(&self, ctx: &FragmentContext) -> PathBuf {
        match &self.memory_md_path {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => ctx.workdir.join(p),
            None => ctx.workdir.join("MEMORY.md"),
        }
    }
}

impl Default for MemoryFragmentProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PromptFragmentProvider for MemoryFragmentProvider {
    fn name(&self) -> &str {
        "memory"
    }

    fn priority(&self) -> u32 {
        4
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        let memory_path = self.resolve_path(ctx);
        let content = load_cached_file_section("memory", &memory_path)?;

        if content.is_empty() {
            return None;
        }

        Some(PromptFragment {
            section_title: "## Memory".to_string(),
            section_type: SectionType::Memory,
            content,
        })
    }

    /// File-backed — keyed by mtime so the builder can skip regeneration.
    fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        let path = self.resolve_path(ctx);
        let meta = std::fs::metadata(&path).ok()?;
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(format!("memory:{}", mtime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_provider_name_and_priority() {
        let provider = MemoryFragmentProvider::new();
        assert_eq!(provider.name(), "memory");
        assert_eq!(provider.priority(), 4);
    }

    #[tokio::test]
    async fn test_generate_no_memory_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_empty_memory_file_returns_none() {
        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_with_memory_content() {
        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "Remember X and Y").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        let frag = fragment.unwrap();
        assert_eq!(frag.section_title, "## Memory");
        assert_eq!(frag.section_type, SectionType::Memory);
        assert_eq!(frag.content, "Remember X and Y");
    }

    #[test]
    fn test_cache_key_none_when_no_memory_file() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        assert!(provider.cache_key(&ctx).is_none());
    }

    #[test]
    fn test_cache_key_contains_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "content").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let key = provider.cache_key(&ctx);
        assert!(key.is_some());
        assert!(key.unwrap().starts_with("memory:"));
    }

    // --- with_path tests ---

    #[test]
    fn test_with_path_stores_relative_path() {
        let provider = MemoryFragmentProvider::with_path("memory/MEMORY.md");
        assert!(provider.memory_md_path.is_some());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_with_custom_relative_path() {
        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        let custom_dir = tmp.path().join("memory");
        fs::create_dir_all(&custom_dir).unwrap();
        fs::write(custom_dir.join("MEMORY.md"), "Custom path content").unwrap();
        let provider = MemoryFragmentProvider::with_path("memory/MEMORY.md");
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        assert_eq!(fragment.unwrap().content, "Custom path content");
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_with_custom_absolute_path() {
        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        let abs_path = tmp.path().join("absolute_MEMORY.md");
        fs::write(&abs_path, "Absolute path content").unwrap();
        let provider = MemoryFragmentProvider::with_path(&abs_path);
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        assert_eq!(fragment.unwrap().content, "Absolute path content");
    }

    #[test]
    fn test_cache_key_with_custom_path() {
        let tmp = tempfile::tempdir().unwrap();
        let custom_dir = tmp.path().join("memory");
        fs::create_dir_all(&custom_dir).unwrap();
        fs::write(custom_dir.join("MEMORY.md"), "content").unwrap();
        let provider = MemoryFragmentProvider::with_path("memory/MEMORY.md");
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let key = provider.cache_key(&ctx);
        assert!(key.is_some());
        assert!(key.unwrap().starts_with("memory:"));
    }

    #[test]
    fn test_cache_key_none_with_custom_path_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MemoryFragmentProvider::with_path("memory/MEMORY.md");
        let ctx = FragmentContext {
            workdir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        assert!(provider.cache_key(&ctx).is_none());
    }
}
