//! Provider for the Memory section of the system prompt.
//!
//! Reads `MEMORY.md` from the agent's working directory and wraps the
//! content as a [`PromptFragment`].

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;
use closeclaw_common::fragment::{
    FragmentContext, PromptFragment, PromptFragmentProvider, SectionType,
};
use closeclaw_common::BootstrapMode;

/// Provider that contributes the long-term memory (`MEMORY.md`) to the
/// system prompt. The file is read from the agent's working directory
/// (or a configured path).
///
/// When the file does not exist or is empty,
/// [`generate`](Self::generate) returns `None`.
pub struct MemoryFragmentProvider {
    /// Configured path to the MEMORY.md file.
    /// When `None`, falls back to `bootstrap_dir.join("MEMORY.md")`.
    memory_md_path: Option<PathBuf>,
}

impl MemoryFragmentProvider {
    /// Create a new provider with no custom path (uses `bootstrap_dir/MEMORY.md`).
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
            Some(p) => ctx.bootstrap_dir.join(p),
            None => ctx.bootstrap_dir.join("MEMORY.md"),
        }
    }
}

impl Default for MemoryFragmentProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Read a file's content if it exists, returning `(content, mtime)`.
///
/// Inlined from `system_prompt::sections::read_file_section` so the
/// memory crate does not depend on `closeclaw-system-prompt`.
fn read_file_section<P: AsRef<Path>>(path: P) -> Option<(String, u64)> {
    let path = path.as_ref();
    let metadata = fs::metadata(path).ok()?;
    let mtime = metadata
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let content = fs::read_to_string(path).ok()?;
    Some((content, mtime))
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
        if ctx.bootstrap_mode == BootstrapMode::Minimal {
            return None;
        }

        let memory_path = self.resolve_path(ctx);
        let (content, _mtime) = read_file_section(&memory_path)?;

        if content.is_empty() {
            return None;
        }

        Some(PromptFragment {
            section_title: "## Memory".to_string(),
            section_type: SectionType::Memory,
            content,
        })
    }

    /// File-backed — keyed by path + mtime so the builder can skip
    /// regeneration. The path hash ensures different workspaces with
    /// identical mtime values produce distinct cache keys.
    fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        let path = self.resolve_path(ctx);
        let meta = std::fs::metadata(&path).ok()?;
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs();
        let mut hasher = DefaultHasher::new();
        path.to_string_lossy().hash(&mut hasher);
        Some(format!("memory:{:x}:{}", hasher.finish(), mtime))
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
            bootstrap_dir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_empty_memory_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_with_memory_content() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "Remember X and Y").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
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
            bootstrap_dir: tmp.path().to_path_buf(),
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
            bootstrap_dir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let key = provider.cache_key(&ctx).unwrap();
        // Format: memory:<path_hash>:<mtime>
        let parts: Vec<&str> = key.split(':').collect();
        assert_eq!(parts[0], "memory");
        assert_eq!(parts.len(), 3, "key should have 3 colon-separated parts");
    }

    #[test]
    fn test_cache_key_unique_per_path() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        fs::write(tmp1.path().join("MEMORY.md"), "same content").unwrap();
        fs::write(tmp2.path().join("MEMORY.md"), "same content").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx1 = FragmentContext {
            bootstrap_dir: tmp1.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let ctx2 = FragmentContext {
            bootstrap_dir: tmp2.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let key1 = provider.cache_key(&ctx1).unwrap();
        let key2 = provider.cache_key(&ctx2).unwrap();
        // Different paths must produce different keys even with same
        // mtime (mtime is not guaranteed identical here, but the path
        // hash component will differ).
        assert_ne!(key1, key2);
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
        let tmp = tempfile::tempdir().unwrap();
        let custom_dir = tmp.path().join("memory");
        fs::create_dir_all(&custom_dir).unwrap();
        fs::write(custom_dir.join("MEMORY.md"), "Custom path content").unwrap();
        let provider = MemoryFragmentProvider::with_path("memory/MEMORY.md");
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        assert_eq!(fragment.unwrap().content, "Custom path content");
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_with_custom_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let abs_path = tmp.path().join("absolute_MEMORY.md");
        fs::write(&abs_path, "Absolute path content").unwrap();
        let provider = MemoryFragmentProvider::with_path(&abs_path);
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
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
            bootstrap_dir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let key = provider.cache_key(&ctx).unwrap();
        let parts: Vec<&str> = key.split(':').collect();
        assert_eq!(parts[0], "memory");
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn test_cache_key_none_with_custom_path_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = MemoryFragmentProvider::with_path("memory/MEMORY.md");
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        assert!(provider.cache_key(&ctx).is_none());
    }

    // --- bootstrap_mode tests ---

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_minimal_mode_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "Remember something").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_full_mode_reads_memory() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MEMORY.md"), "Full mode memory").unwrap();
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Full,
            ..FragmentContext::test_default()
        };
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        let frag = fragment.unwrap();
        assert_eq!(frag.section_title, "## Memory");
        assert_eq!(frag.section_type, SectionType::Memory);
        assert_eq!(frag.content, "Full mode memory");
    }

    #[tokio::test]
    async fn test_generate_no_workspace_dir_returns_none() {
        let provider = MemoryFragmentProvider::new();
        let ctx = FragmentContext {
            bootstrap_dir: PathBuf::from("/nonexistent/path/to/workspace"),
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_generate_with_path_and_minimal_mode_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let abs_path = tmp.path().join("MEMORY.md");
        fs::write(&abs_path, "Should not be read").unwrap();
        let provider = MemoryFragmentProvider::with_path(&abs_path);
        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
            bootstrap_mode: BootstrapMode::Minimal,
            ..FragmentContext::test_default()
        };
        assert!(provider.generate(&ctx).await.is_none());
    }
}
