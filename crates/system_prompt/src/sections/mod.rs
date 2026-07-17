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

use closeclaw_common::{ModeTransition, PlanPath, SessionMode};

mod mode_prompts;

#[cfg(test)]
mod mode_prompts_tests;
#[cfg(test)]
mod sections_tests;

use self::mode_prompts::*;

#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// not Normal. For Plan mode, `plan_path` determines which
    /// path-specific instruction to inject.
    ModeInstruction {
        mode: SessionMode,
        plan_path: Option<PlanPath>,
        sparse: bool,
        sub_agent: bool,
    },
    /// Mode transition notification, injected when the session
    /// transitions between modes (e.g. re-entry, exit).
    ModeTransition {
        transition: ModeTransition,
    },
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
            Section::ModeInstruction { .. } => "mode_instruction",
            Section::ModeTransition { .. } => "mode_transition",
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
            Section::ModeInstruction {
                mode,
                plan_path,
                sparse,
                sub_agent,
            } => render_mode_instruction_with_flags(*mode, *plan_path, *sparse, *sub_agent),
            Section::ModeTransition { transition } => render_mode_transition(transition),
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
fn render_mode_instruction(mode: SessionMode, plan_path: Option<PlanPath>) -> String {
    match mode {
        SessionMode::Normal => String::new(),
        SessionMode::Plan => {
            let path = plan_path.unwrap_or_default();
            match path {
                PlanPath::Standard => render_standard_path_instruction(),
                PlanPath::Interview => render_interview_path_instruction(),
            }
        }
        SessionMode::Auto => {
            format!("## Mode: Auto\n\n{}\n", AUTO_MODE_PROMPT)
        }
    }
}

/// Render mode instruction with sparse/sub-agent variant selection.
///
/// When `sparse` is true, returns the appropriate sparse text.
/// When `sub_agent` is true, returns the sub-agent sparse text.
/// Otherwise delegates to the full `render_mode_instruction`.
pub(crate) fn render_mode_instruction_with_flags(
    mode: SessionMode,
    plan_path: Option<PlanPath>,
    sparse: bool,
    sub_agent: bool,
) -> String {
    if sub_agent {
        return SUBAGENT_SPARSE.to_string();
    }
    if sparse {
        return match mode {
            SessionMode::Auto => AUTO_MODE_SPARSE.to_string(),
            _ => STANDARD_SPARSE.to_string(),
        };
    }
    render_mode_instruction(mode, plan_path)
}

/// Render a mode transition prompt — design doc section 6.
fn render_mode_transition(transition: &ModeTransition) -> String {
    match transition {
        ModeTransition::Reentry => MODE_REENTRY.to_string(),
        ModeTransition::ExitPlan => MODE_EXIT_PLAN.to_string(),
        ModeTransition::ExitAuto => MODE_EXIT_AUTO.to_string(),
    }
}

/// Render Standard Path instructions (5 Phases).
///
/// Uses verbatim prompt content from design doc section 1 (global
/// constraint) and section 2 (Phase 1–5 including Submit for
/// Approval).
fn render_standard_path_instruction() -> String {
    format!(
        "## Mode: Plan \u{2014} Standard Path\n\n{}\n\n{}\n",
        PLAN_MODE_CONSTRAINT, STANDARD_PATH_PHASES
    )
}

/// Render Interview Path instructions.
///
/// Used when the user request is ambiguous and requires iterative
/// exploration and clarification before a plan can be formed.
/// Content verbatim from design doc section 3.
fn render_interview_path_instruction() -> String {
    format!(
        "## Mode: Plan \u{2014} Interview Path\n\n{}\n\n{}\n",
        PLAN_MODE_CONSTRAINT, INTERVIEW_PATH_PROMPT
    )
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
