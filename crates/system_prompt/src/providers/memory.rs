//! Provider for the Memory section of the system prompt.
//!
//! Reads `MEMORY.md` from the agent's working directory and wraps the
//! content as a [`PromptFragment`].

use async_trait::async_trait;

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};
use crate::sections::load_cached_file_section;

/// Provider that contributes the long-term memory (`MEMORY.md`) to the
/// system prompt. The file is read from the agent's working directory.
///
/// When the file does not exist or is empty,
/// [`generate`](Self::generate) returns `None`.
pub struct MemoryFragmentProvider;

impl MemoryFragmentProvider {
    pub fn new() -> Self {
        Self
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
        let workdir = ctx.workdir.as_ref()?;

        let memory_path = workdir.join("MEMORY.md");
        let content = load_cached_file_section("memory", &memory_path)?;

        if content.is_empty() {
            return None;
        }

        Some(PromptFragment {
            title: "## Memory".to_string(),
            section_type: SectionType::Memory,
            content,
        })
    }

    /// File-backed — keyed by mtime so the builder can skip regeneration.
    fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        let workdir = ctx.workdir.as_ref()?;
        let path = workdir.join("MEMORY.md");
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
    async fn test_generate_no_workdir_returns_none() {
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext::default();
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_no_memory_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_empty_memory_file_returns_none() {
        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_with_memory_content() {
        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "Remember X and Y").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        let frag = fragment.unwrap();
        assert_eq!(frag.title, "## Memory");
        assert_eq!(frag.section_type, SectionType::Memory);
        assert_eq!(frag.content, "Remember X and Y");
    }

    #[test]
    fn test_cache_key_none_when_no_workdir() {
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext::default();
        assert!(provider.cache_key(&ctx).is_none());
    }

    #[test]
    fn test_cache_key_none_when_no_memory_file() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        assert!(provider.cache_key(&ctx).is_none());
    }

    #[test]
    fn test_cache_key_contains_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "content").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let key = provider.cache_key(&ctx);
        assert!(key.is_some());
        assert!(key.unwrap().starts_with("memory:"));
    }
}
