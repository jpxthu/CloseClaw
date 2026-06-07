//! Section definitions and caching for System Prompt building
//!
//! Sections are divided into STATIC (cached, rebuilt only on invalidation) and
//! DYNAMIC (rebuilt on every buildSystemPrompt call).

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::RwLock;
use std::time::SystemTime;

/// Represents a system prompt section
#[derive(Debug, Clone)]
pub enum Section {
    // --- Static sections (cached) ---
    RoleSection(String),
    ToolsSection(String),
    MemorySection(String),
    HeartbeatSection(String),
    SkillListingSection(String),
    // --- Dynamic sections (always rebuilt) ---
    ChannelContext {
        chat_name: String,
        sender_id: String,
        timestamp: String,
    },
    SessionState {
        pending_tasks: Vec<String>,
    },
    AppendSection(String),
    GitStatus(String),
    WorkingDirectory(String),
}

impl Section {
    /// Returns true if this section is cacheable (static)
    pub fn is_cacheable(&self) -> bool {
        matches!(
            self,
            Section::RoleSection(_)
                | Section::MemorySection(_)
                | Section::HeartbeatSection(_)
                | Section::SkillListingSection(_)
        )
    }

    /// Returns the section name for cache key purposes
    pub fn name(&self) -> &'static str {
        match self {
            Section::RoleSection(_) => "role",
            Section::ToolsSection(_) => "tools",
            Section::MemorySection(_) => "memory",
            Section::HeartbeatSection(_) => "heartbeat",
            Section::SkillListingSection(_) => "skill_listing",
            Section::ChannelContext { .. } => "channel_context",
            Section::SessionState { .. } => "session_state",
            Section::AppendSection(_) => "append",
            Section::GitStatus(_) => "git_status",
            Section::WorkingDirectory(_) => "working_directory",
        }
    }
    fn format_session_state(&self, pending_tasks: &[String]) -> String {
        let tasks_str = if pending_tasks.is_empty() {
            "  (none)".to_string()
        } else {
            pending_tasks
                .iter()
                .enumerate()
                .map(|(i, t)| format!("  {}. {}", i + 1, t))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!("## Session State\n- pending_tasks:\n{}\n", tasks_str)
    }

    /// Render the section as a string for the system prompt
    pub fn render(&self) -> String {
        match self {
            Section::RoleSection(content) => {
                format!("## Role\n{}\n", content)
            }
            Section::ToolsSection(content) => {
                format!("## Tools\n{}\n", content)
            }
            Section::MemorySection(content) => {
                format!("## Memory\n{}\n", content)
            }
            Section::HeartbeatSection(content) => {
                format!("## Heartbeat Context\n{}\n", content)
            }
            Section::SkillListingSection(content) => {
                if content.is_empty() {
                    String::new()
                } else {
                    format!("## Available Skills\n\n{}\n", content)
                }
            }
            Section::ChannelContext {
                chat_name,
                sender_id,
                timestamp,
            } => {
                format!(
                    "## Channel Context\n- chat_name: {}\n- sender_id: {}\n- timestamp: {}\n",
                    chat_name, sender_id, timestamp
                )
            }
            Section::SessionState { pending_tasks } => self.format_session_state(pending_tasks),
            Section::AppendSection(content) => {
                format!("## Append\n{}\n", content)
            }
            Section::GitStatus(content) => {
                format!("## Git Status\n{}\n", content)
            }
            Section::WorkingDirectory(path) => {
                let sanitized = sanitize_workdir_path(path);
                format!("## Working Directory\n当前工作目录：{}\n", sanitized)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Section Cache
// ---------------------------------------------------------------------------

/// Entry stored in the section cache
#[derive(Debug, Clone)]
struct CacheEntry {
    content: String,
    file_mtime: Option<u64>,
}

/// Process-wide section cache
static SECTION_CACHE: LazyLock<RwLock<HashMap<String, CacheEntry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Get a cached section if still valid (mtime matches)
pub fn get_cached_section(name: &str, current_mtime: Option<u64>) -> Option<String> {
    let cache = SECTION_CACHE.read().ok()?;
    let entry = cache.get(name)?;

    // If mtime was provided, validate it matches
    if let (Some(cached_mtime), Some(current)) = (entry.file_mtime, current_mtime) {
        if cached_mtime != current {
            return None; // stale
        }
    }

    Some(entry.content.clone())
}

/// Put a section into the cache (public for use by builder)
pub fn put_cached_section(name: &str, content: String, file_mtime: Option<u64>) {
    if let Ok(mut cache) = SECTION_CACHE.write() {
        cache.insert(
            name.to_string(),
            CacheEntry {
                content,
                file_mtime,
            },
        );
    }
}

/// Manually invalidate a named section
pub fn invalidate_section(name: &str) {
    if let Ok(mut cache) = SECTION_CACHE.write() {
        cache.remove(name);
    }
}

/// Invalidate all cached sections
pub fn invalidate_all_sections() {
    if let Ok(mut cache) = SECTION_CACHE.write() {
        cache.clear();
    }
}

/// Invalidate the skill listing section cache.
///
/// Call this when skill files change so the next system prompt build
/// regenerates the listing from the current registry state.
pub fn invalidate_skill_listing() {
    invalidate_section("skill_listing");
}

// ---------------------------------------------------------------------------
// File-based section helpers
// ---------------------------------------------------------------------------

/// Read a file's content if it exists, returning (content, mtime)
pub fn read_file_section<P: AsRef<Path>>(path: P) -> Option<(String, u64)> {
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

/// Load and cache a static file-based section
/// Returns cached value if mtime unchanged; otherwise reloads and caches.
pub fn load_cached_file_section(name: &str, path: &Path) -> Option<String> {
    let (content, mtime) = read_file_section(path)?;

    if let Some(cached) = get_cached_section(name, Some(mtime)) {
        return Some(cached);
    }

    // Cache miss or stale — store and return
    put_cached_section(name, content.clone(), Some(mtime));
    Some(content)
}

// ---------------------------------------------------------------------------
// Working Directory sanitization
// ---------------------------------------------------------------------------

/// Strip path prefix up to and including `workspaces/`, prepend `~/`.
/// If the path doesn't contain `workspaces/`, return unchanged.
pub fn sanitize_workdir_path(path: &str) -> String {
    if let Some(idx) = path.find("workspaces/") {
        format!("~/{}", &path[idx + "workspaces/".len()..])
    } else {
        path.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    #[test]
    fn test_section_render_role() {
        let s = Section::RoleSection("You are a helpful assistant.".to_string());
        let rendered = s.render();
        assert!(rendered.contains("## Role"));
        assert!(rendered.contains("You are a helpful assistant"));
        assert!(s.is_cacheable());
    }

    #[test]
    fn test_section_render_channel_context() {
        let s = Section::ChannelContext {
            chat_name: "test-chat".to_string(),
            sender_id: "ou_123".to_string(),
            timestamp: "2026-04-10T15:00:00+08:00".to_string(),
        };
        let rendered = s.render();
        assert!(rendered.contains("chat_name: test-chat"));
        assert!(rendered.contains("sender_id: ou_123"));
        assert!(!s.is_cacheable());
    }

    #[test]
    fn test_section_render_session_state() {
        let s = Section::SessionState {
            pending_tasks: vec!["task1".to_string(), "task2".to_string()],
        };
        let rendered = s.render();
        assert!(rendered.contains("task1"));
        assert!(!s.is_cacheable());
    }

    #[test]
    fn test_invalidate_section() {
        put_cached_section("test_section", "old content".to_string(), Some(100));
        assert_eq!(
            get_cached_section("test_section", Some(100)),
            Some("old content".to_string())
        );

        invalidate_section("test_section");
        assert_eq!(get_cached_section("test_section", Some(100)), None);
    }

    #[test]
    fn test_cache_stale_on_mtime_change() {
        put_cached_section("file_section", "v1".to_string(), Some(100));
        // Same mtime → cache hit
        assert_eq!(
            get_cached_section("file_section", Some(100)),
            Some("v1".to_string())
        );
        // Different mtime → cache stale
        assert_eq!(get_cached_section("file_section", Some(200)), None);
    }

    #[test]
    fn test_load_cached_file_section_fresh() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        // First load — cache miss, should read from file
        let result = load_cached_file_section("test", &file_path);
        assert_eq!(result, Some("hello world".to_string()));

        // Second load — cache hit, same content
        let result2 = load_cached_file_section("test", &file_path);
        assert_eq!(result2, Some("hello world".to_string()));

        // Modify file — cache should be stale
        // Sleep 1s to ensure mtime changes (filesystem mtime resolution is 1s)
        std::thread::sleep(std::time::Duration::from_secs(1));
        std::fs::write(&file_path, "updated content").unwrap();
        let result3 = load_cached_file_section("test", &file_path);
        assert_eq!(result3, Some("updated content".to_string()));
    }

    #[test]
    fn test_skill_listing_section_is_cacheable() {
        let s = Section::SkillListingSection("some skills".to_string());
        assert!(s.is_cacheable());
    }

    #[test]
    fn test_skill_listing_section_name() {
        let s = Section::SkillListingSection("some skills".to_string());
        assert_eq!(s.name(), "skill_listing");
    }

    #[test]
    fn test_skill_listing_section_render_format() {
        let s = Section::SkillListingSection(
            "- **foo**: desc — use when needed
- **bar**: desc"
                .to_string(),
        );
        let rendered = s.render();
        assert!(rendered.starts_with("## Available Skills\n\n"));
        assert!(rendered.contains("**foo**"));
        assert!(rendered.contains("**bar**"));
        assert!(rendered.contains(" — use when needed"));
        assert!(rendered.ends_with("\n"));
    }

    #[test]
    fn test_skill_listing_section_render_empty() {
        let s = Section::SkillListingSection(String::new());
        assert_eq!(s.render(), "");
    }

    #[test]
    fn test_git_status_render() {
        let s = Section::GitStatus("On branch master\n?? file.txt".to_string());
        let rendered = s.render();
        assert!(rendered.contains("## Git Status"));
        assert!(rendered.contains("On branch master"));
        assert!(!s.is_cacheable());
    }

    #[test]
    fn test_working_directory_section() {
        let s =
            Section::WorkingDirectory("/home/user/.closeclaw/workspaces/agent1/user1/".to_string());
        assert!(!s.is_cacheable());
        assert_eq!(s.name(), "working_directory");
        let rendered = s.render();
        assert!(rendered.contains("## Working Directory"));
        assert!(rendered.contains("~/agent1/user1/"));
        assert!(!rendered.contains(".closeclaw"));
    }

    #[test]
    fn test_sanitize_workdir_path() {
        assert_eq!(
            sanitize_workdir_path("/home/user/.closeclaw/workspaces/a/u/"),
            "~/a/u/"
        );
        assert_eq!(
            sanitize_workdir_path("/some/random/path"),
            "/some/random/path"
        );
        assert_eq!(sanitize_workdir_path(""), "");
    }

    #[test]
    fn test_invalidate_skill_listing() {
        // Pre-populate the skill_listing cache with known content
        put_cached_section("skill_listing", "old skill content".to_string(), Some(999));
        // Verify it's cached
        assert_eq!(
            get_cached_section("skill_listing", Some(999)),
            Some("old skill content".to_string())
        );

        // Invalidate via the public API
        invalidate_skill_listing();

        // Cache should be cleared
        assert_eq!(get_cached_section("skill_listing", Some(999)), None);
    }

    // -----------------------------------------------------------------------
    // Coverage for all remaining Section variants after WorkspaceSection removal
    // -----------------------------------------------------------------------

    #[test]
    fn test_role_section_name() {
        let s = Section::RoleSection("You are a helpful assistant.".to_string());
        assert_eq!(s.name(), "role");
    }

    #[test]
    fn test_tools_section() {
        let s = Section::ToolsSection("tool_a\ntool_b".to_string());
        assert!(!s.is_cacheable());
        assert_eq!(s.name(), "tools");
        let rendered = s.render();
        assert!(rendered.starts_with("## Tools\n"));
        assert!(rendered.contains("tool_a"));
    }

    #[test]
    fn test_memory_section() {
        let s = Section::MemorySection("remember X".to_string());
        assert!(s.is_cacheable());
        assert_eq!(s.name(), "memory");
        let rendered = s.render();
        assert!(rendered.starts_with("## Memory\n"));
        assert!(rendered.contains("remember X"));
    }

    #[test]
    fn test_heartbeat_section() {
        let s = Section::HeartbeatSection("hb data".to_string());
        assert!(s.is_cacheable());
        assert_eq!(s.name(), "heartbeat");
        let rendered = s.render();
        assert!(rendered.starts_with("## Heartbeat Context\n"));
        assert!(rendered.contains("hb data"));
    }

    #[test]
    fn test_channel_context_name() {
        let s = Section::ChannelContext {
            chat_name: "c".to_string(),
            sender_id: "s".to_string(),
            timestamp: "t".to_string(),
        };
        assert_eq!(s.name(), "channel_context");
    }

    #[test]
    fn test_session_state_name() {
        let s = Section::SessionState {
            pending_tasks: vec![],
        };
        assert_eq!(s.name(), "session_state");
    }

    #[test]
    fn test_append_section() {
        let s = Section::AppendSection("extra info".to_string());
        assert!(!s.is_cacheable());
        assert_eq!(s.name(), "append");
        let rendered = s.render();
        assert!(rendered.starts_with("## Append\n"));
        assert!(rendered.contains("extra info"));
    }

    #[test]
    fn test_git_status_name() {
        let s = Section::GitStatus("On branch main".to_string());
        assert_eq!(s.name(), "git_status");
    }
}
