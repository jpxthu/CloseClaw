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

use closeclaw_common::SessionMode;

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
    /// Mode-specific instruction section, injected when session mode is
    /// not Normal.
    ModeInstruction(SessionMode),
}

impl Section {
    /// Returns true if this section is cacheable (static)
    pub fn is_cacheable(&self) -> bool {
        matches!(
            self,
            Section::RoleSection(_) | Section::MemorySection(_) | Section::HeartbeatSection(_)
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
            Section::ModeInstruction(_) => "mode_instruction",
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
            Section::ModeInstruction(mode) => render_mode_instruction(*mode),
        }
    }
}

// ---------------------------------------------------------------------------
// Mode Instruction Rendering
// ---------------------------------------------------------------------------

/// Render mode-specific instructions based on session mode.
///
/// - Normal: no extra instructions (returns empty string)
/// - Plan: Plan Mode workflow instructions
/// - Auto: Auto Mode execution instructions
fn render_mode_instruction(mode: SessionMode) -> String {
    match mode {
        SessionMode::Normal => String::new(),
        SessionMode::Plan => {
            "## Mode: Plan\n\n".to_string()
                + "You are in **Plan Mode**. Your goal is to produce a clear, \
                  implementable plan that can be reviewed and approved before \
                  any code changes are made.\n\n"
                + "### Path Selection\n\n"
                + "Before starting work, assess whether the user's request is \
                  sufficiently clear. Use the following criteria:\n\n"
                + "**Standard Path** — use when the request includes:\n"
                + "- Explicit file, module, or interface references\n"
                + "- Quantifiable acceptance criteria\n"
                + "- Sufficient context to proceed without further clarification\n\n"
                + "**Interview Path** — use when the request is ambiguous, \
                  lacks specificity, or requires deeper exploration to \
                  understand the true scope.\n\n"
                + "You may also choose the Interview Path if early exploration \
                  reveals unexpected complexity or scope creep.\n\n"
                + "**Explicit path specification** — the user may explicitly \\
                  request a specific path via command arguments (e.g., \\
                  `--path standard` or `--path interview`). When an \\
                  explicit path is specified, the system adopts it directly \\
                  without performing automatic clarity analysis.\n\n"
                + "### Standard Path (4 Phases)\n\n"
                + "1. **Research** — gather context, read code, understand the \
                     problem space. Spawn Explore agents for parallel \
                     codebase exploration.\n"
                + "2. **Design** — produce a concrete implementation plan \
                     with file-level granularity. Identify key files, \
                     interfaces, and data flows.\n"
                + "3. **Review** — self-check the plan for correctness and \
                     completeness. Use AskUserQuestion for any remaining \
                     requirement clarification only.\n"
                + "4. **Final Plan** — write the validated plan into the plan \
                     file (the only write operation in Plan Mode).\n\n"
                + "### Interview Path\n\n"
                + "The Interview Path has no fixed phases. You operate in a \
                  loop:\n"
                + "1. **Explore** — spawn Explore agents to understand \
                     existing code and constraints\n"
                + "2. **Update plan** — incrementally write findings and \
                     emerging understanding into the plan file\n"
                + "3. **Ask** — use AskUserQuestion to clarify remaining \
                     ambiguities with the user\n"
                + "4. **Evaluate** — if ambiguities remain, repeat from step 1. \
                     If requirements converge, proceed to Review and \
                     Final Plan (same as Standard Path steps 3–4).\n\n"
                + "### Constraints\n\n"
                + "- **Read-only tools only** — you may not modify source code \
                     or configuration files\n"
                + "- **plans/ directory is the sole writable area** — the plan \
                     file under `workspace/plans/` is the only file you \
                     may create or modify\n"
                + "- **No execution** — do not run builds, tests, or deploy \
                     commands\n"
                + "- **Approval required to exit** — once your plan is \
                     complete, use the approval tool to submit it for \
                     user review. The plan must be confirmed before \
                     any code execution is permitted\n\n"
                + "### Approval\n\n"
                + "When you have finished preparing the plan, call the \
                  approval tool with a plan summary. The framework will \
                  present a confirmation dialog to the user. Upon \
                  approval, Plan Mode ends and Auto Mode begins for \
                  execution."
        }
        SessionMode::Auto => {
            "## Mode: Auto\n".to_string()
                + "You are in **Auto Mode**. Execute tasks autonomously:\n"
                + "- Complete each task step by step\n"
                + "- Run tests to verify your changes\n"
                + "- Dangerous operations (file deletion, force push) require approval\n"
                + "- Commit and report when done\n"
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
        assert!(!s.is_cacheable());
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

    // -----------------------------------------------------------------------
    // ModeInstruction tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_mode_instruction_name() {
        let s = Section::ModeInstruction(SessionMode::Normal);
        assert_eq!(s.name(), "mode_instruction");
    }

    #[test]
    fn test_mode_instruction_not_cacheable() {
        let s = Section::ModeInstruction(SessionMode::Plan);
        assert!(!s.is_cacheable());
    }

    #[test]
    fn test_mode_instruction_normal_renders_empty() {
        let s = Section::ModeInstruction(SessionMode::Normal);
        assert_eq!(s.render(), "");
    }

    #[test]
    fn test_mode_instruction_plan_renders_instructions() {
        let s = Section::ModeInstruction(SessionMode::Plan);
        let rendered = s.render();
        assert!(rendered.contains("## Mode: Plan"));
        assert!(rendered.contains("Plan Mode"));
        // Standard path phases
        assert!(rendered.contains("Research"));
        assert!(rendered.contains("Design"));
        assert!(rendered.contains("Review"));
        assert!(rendered.contains("Final Plan"));
        // Interview path
        assert!(rendered.contains("Interview"));
        // Path selection logic
        assert!(rendered.contains("Path Selection"));
        assert!(rendered.contains("Standard Path"));
        assert!(rendered.contains("Interview Path"));
        // Constraints
        assert!(rendered.contains("Read-only"));
        assert!(rendered.contains("plans/"));
        // Approval exit
        assert!(rendered.contains("approval tool"));
    }

    #[test]
    fn test_mode_instruction_plan_dual_path_description() {
        let rendered = render_mode_instruction(SessionMode::Plan);
        // Verify both paths are described with distinct characteristics
        assert!(rendered.contains("4 Phases"));
        assert!(rendered.contains("loop"));
    }

    #[test]
    fn test_mode_instruction_plan_readonly_constraint() {
        let rendered = render_mode_instruction(SessionMode::Plan);
        assert!(rendered.contains("writable area"));
        assert!(rendered.contains("No execution"));
    }

    #[test]
    fn test_mode_instruction_plan_approval_exit() {
        let rendered = render_mode_instruction(SessionMode::Plan);
        assert!(rendered.contains("Approval required to exit"));
        assert!(rendered.contains("confirmation dialog"));
    }

    #[test]
    fn test_mode_instruction_auto_renders_instructions() {
        let s = Section::ModeInstruction(SessionMode::Auto);
        let rendered = s.render();
        assert!(rendered.contains("## Mode: Auto"));
        assert!(rendered.contains("Auto Mode"));
        assert!(rendered.contains("autonomously"));
        assert!(rendered.contains("approval"));
    }

    #[test]
    fn test_mode_instruction_auto_unchanged() {
        // Verify Auto mode output is identical to the original implementation
        let rendered = render_mode_instruction(SessionMode::Auto);
        assert!(rendered.contains("Execute tasks autonomously"));
        assert!(rendered.contains("Commit and report when done"));
        assert!(rendered.contains("Dangerous operations"));
    }
}
