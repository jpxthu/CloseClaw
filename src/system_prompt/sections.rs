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

/// Maximum length of append_section content (in characters)
pub const APPEND_SECTION_MAX_LEN: usize = 500;

/// Represents a system prompt section
#[derive(Debug, Clone)]
pub enum Section {
    // --- Static sections (cached) ---
    RoleSection(String),
    WorkspaceSection(String),
    ToolsSection(String),
    MemorySection(String),
    HeartbeatSection(String),
    // --- Dynamic sections (always rebuilt) ---
    ChannelContext {
        chat_name: String,
        sender_id: String,
        timestamp: String,
    },
    SessionState {
        turn_count: u32,
        pending_tasks: Vec<String>,
    },
    AppendSection(String),
    GitStatus(String),
}

impl Section {
    /// Returns true if this section is cacheable (static)
    pub fn is_cacheable(&self) -> bool {
        matches!(
            self,
            Section::RoleSection(_)
                | Section::WorkspaceSection(_)
                | Section::ToolsSection(_)
                | Section::MemorySection(_)
                | Section::HeartbeatSection(_)
        )
    }

    /// Returns the section name for cache key purposes
    pub fn name(&self) -> &'static str {
        match self {
            Section::RoleSection(_) => "role",
            Section::WorkspaceSection(_) => "workspace",
            Section::ToolsSection(_) => "tools",
            Section::MemorySection(_) => "memory",
            Section::HeartbeatSection(_) => "heartbeat",
            Section::ChannelContext { .. } => "channel_context",
            Section::SessionState { .. } => "session_state",
            Section::AppendSection(_) => "append",
            Section::GitStatus(_) => "git_status",
        }
    }

    /// Render the section as a string for the system prompt
    pub fn render(&self) -> String {
        match self {
            Section::RoleSection(content) => {
                format!("## Role\n{}\n", content)
            }
            Section::WorkspaceSection(content) => {
                format!("## Workspace\n{}\n", content)
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
            Section::SessionState {
                turn_count,
                pending_tasks,
            } => {
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
                format!(
                    "## Session State\n- turn_count: {}\n- pending_tasks:\n{}\n",
                    turn_count, tasks_str
                )
            }
            Section::AppendSection(content) => {
                format!("## Append\n{}\n", content)
            }
            Section::GitStatus(content) => {
                format!("## Git Status\n{}\n", content)
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
// Append Section (request-scoped)
// ---------------------------------------------------------------------------

/// Append section state — NOT persisted, cleared after each request
static APPEND_SECTION: RwLock<Option<String>> = RwLock::new(None);

/// Set the append section content, truncating if over MAX_LEN and returning
/// a notification message.
pub fn set_append_section(text: String) -> Option<String> {
    let is_truncated = text.chars().count() > APPEND_SECTION_MAX_LEN;
    let warning;
    let content = if is_truncated {
        let chars: Vec<char> = text.chars().take(APPEND_SECTION_MAX_LEN).collect();
        warning = Some("⚠️ 内容已截断至 500 字限制".to_string());
        chars.iter().collect::<String>()
    } else {
        warning = None;
        text.clone()
    };

    if let Ok(mut guard) = APPEND_SECTION.write() {
        *guard = Some(content);
    }

    warning
}

/// Get the current append section content
pub fn get_append_section() -> Option<String> {
    APPEND_SECTION.write().ok()?.clone()
}

/// Clear the append section (called after request completes)
pub fn clear_append_section() {
    if let Ok(mut guard) = APPEND_SECTION.write() {
        *guard = None;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
            turn_count: 5,
            pending_tasks: vec!["task1".to_string(), "task2".to_string()],
        };
        let rendered = s.render();
        assert!(rendered.contains("turn_count: 5"));
        assert!(rendered.contains("task1"));
        assert!(!s.is_cacheable());
    }

    #[test]

    fn test_append_section_truncation() {
        let long_text = "a".repeat(600);
        let warning = set_append_section(long_text);
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("500"));

        let content = get_append_section().unwrap();
        assert_eq!(content.chars().count(), APPEND_SECTION_MAX_LEN);
    }

    #[test]

    fn test_append_section_no_truncation() {
        let text = "short text".to_string();
        let warning = set_append_section(text.clone());
        assert!(warning.is_none());
        assert_eq!(get_append_section(), Some(text));
    }

    #[test]

    fn test_append_section_cleared_after_request() {
        set_append_section("test".to_string());
        assert!(get_append_section().is_some());
        clear_append_section();
        assert!(get_append_section().is_none());
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
    fn test_git_status_render() {
        let s = Section::GitStatus("On branch master\n?? file.txt".to_string());
        let rendered = s.render();
        assert!(rendered.contains("## Git Status"));
        assert!(rendered.contains("On branch master"));
        assert!(!s.is_cacheable());
    }
}
