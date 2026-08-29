//! Section definitions and caching for System Prompt building
//!
//! Sections are divided into STATIC (cached, rebuilt only on invalidation) and
//! DYNAMIC (rebuilt on every buildSystemPrompt call).

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::system_prompt::ModeTransition;
use closeclaw_execution::PlanPath;

mod mode_prompts;

#[cfg(test)]
mod mode_prompts_tests;
#[cfg(test)]
mod sections_tests;

use self::mode_prompts::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    // --- Static sections (cached) ---
    ToolsSection(String),
    MemorySection(String),
    // --- Dynamic sections (always rebuilt) ---
    ChannelContext {
        chat_name: String,
    },
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
    /// Mode transition prompt, injected when a session mode change occurs.
    /// Carries the transition type and renders the corresponding prompt
    /// from design doc §6.
    ModeTransition(ModeTransition),
    /// Plan file context for Auto Mode injection.
    ///
    /// Contains the plan file path and its full content, injected
    /// after ModeInstruction in Auto Mode.
    PlanFile {
        path: String,
        content: String,
    },
}

impl Section {
    /// Returns true if this section is cacheable (static)
    pub fn is_cacheable(&self) -> bool {
        matches!(self, Section::ToolsSection(_) | Section::MemorySection(_))
    }

    /// Returns the section name for cache key purposes
    pub fn name(&self) -> &'static str {
        match self {
            Section::ToolsSection(_) => "tools",
            Section::MemorySection(_) => "memory",

            Section::ChannelContext { .. } => "channel_context",
            Section::GitStatus(_) => "git_status",
            Section::WorkingDirectory(_) => "working_directory",
            Section::ModeInstruction { .. } => "mode_instruction",
            Section::ModeTransition(_) => "mode_transition",
            Section::PlanFile { .. } => "plan_file",
        }
    }
    /// Render the section as a string for the system prompt
    pub fn render(&self) -> String {
        match self {
            Section::ToolsSection(content) => {
                format!("## Tools\n{}\n", content)
            }
            Section::MemorySection(content) => {
                format!("## Memory\n{}\n", content)
            }

            Section::ChannelContext { chat_name } => {
                format!("## Channel Context\n- chat_name: {}\n", chat_name)
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
            Section::ModeTransition(transition) => render_mode_transition(*transition),
            Section::PlanFile { path, content } => {
                format!("## Plan File\n路径：{}\n\n{}\n", path, content)
            }
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
        SessionMode::Plan => match plan_path {
            Some(PlanPath::Standard) => render_standard_path_instruction(),
            Some(PlanPath::Interview) => render_interview_path_instruction(),
            None => render_plan_mode_instruction(),
        },
        SessionMode::Auto => {
            format!("## Mode: Auto\n\n{}\n", AUTO_MODE_PROMPT)
        }
    }
}

/// Render mode instruction with sparse/sub-agent variant selection.
///
/// When `sparse` is true, returns the appropriate sparse text.
/// When `sub_agent` is true **and** `mode` is `Plan`, returns the
/// sub-agent sparse text. For other modes, `sub_agent` is ignored and
/// the normal mode rendering logic applies.
/// Otherwise delegates to the full `render_mode_instruction`.
pub(crate) fn render_mode_instruction_with_flags(
    mode: SessionMode,
    plan_path: Option<PlanPath>,
    sparse: bool,
    sub_agent: bool,
) -> String {
    if sub_agent && mode == SessionMode::Plan {
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

/// Render Plan Mode instructions with path selection rules.
///
/// When no explicit path is specified, injects §1 global constraint,
/// §1 path selection rules, §2 standard path, and §3 interview path.
/// The Agent reads the task description and decides which path to follow.
fn render_plan_mode_instruction() -> String {
    format!(
        "## Mode: Plan\n\n{}\n\n{}\n\n{}\n\n{}\n",
        PLAN_MODE_CONSTRAINT, PATH_SELECTION_RULES, STANDARD_PATH_PHASES, INTERVIEW_PATH_PROMPT
    )
}

/// Render Standard Path instructions (4 Phases).
///
/// Uses verbatim prompt content from design doc section 1 (global
/// constraint) and section 2 (Phase 1–4).
fn render_standard_path_instruction() -> String {
    format!("{}\n\n{}\n", PLAN_MODE_CONSTRAINT, STANDARD_PATH_PHASES)
}

/// Render Interview Path instructions.
///
/// Used when the user request is ambiguous and requires iterative
/// exploration and clarification before a plan can be formed.
/// Content verbatim from design doc section 3.
fn render_interview_path_instruction() -> String {
    format!("{}\n\n{}\n", PLAN_MODE_CONSTRAINT, INTERVIEW_PATH_PROMPT)
}

/// Render mode transition prompt based on transition type.
///
/// Content verbatim from design doc section 6.
fn render_mode_transition(transition: ModeTransition) -> String {
    match transition {
        ModeTransition::PlanModeReentry => PLAN_MODE_REENTRY.to_string(),
        ModeTransition::PlanModeExit => PLAN_MODE_EXIT.to_string(),
        ModeTransition::AutoModeExit => AUTO_MODE_EXIT.to_string(),
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

/// Session-scoped section cache.
///
/// Each [`PromptBuilder`](crate::builder::PromptBuilder) instance holds
/// its own `SectionCache`, ensuring per-session isolation. This replaces
/// the former process-wide `static SECTION_CACHE`.
pub struct SectionCache {
    entries: HashMap<String, CacheEntry>,
}

impl Default for SectionCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SectionCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Get a cached section if still valid (mtime matches).
    pub fn get(&self, name: &str, current_mtime: Option<u64>) -> Option<String> {
        let entry = self.entries.get(name)?;

        // If mtime was provided, validate it matches
        if let (Some(cached_mtime), Some(current)) = (entry.file_mtime, current_mtime) {
            if cached_mtime != current {
                return None; // stale
            }
        }

        Some(entry.content.clone())
    }

    /// Put a section into the cache.
    pub fn put(&mut self, name: &str, content: String, file_mtime: Option<u64>) {
        self.entries.insert(
            name.to_string(),
            CacheEntry {
                content,
                file_mtime,
            },
        );
    }

    /// Invalidate (remove) a single named section.
    pub fn invalidate(&mut self, name: &str) {
        self.entries.remove(name);
    }

    /// Invalidate all cached sections.
    pub fn invalidate_all(&mut self) {
        self.entries.clear();
    }

    /// Invalidate the tools section cache.
    ///
    /// Call this when tool definitions change (e.g. a new tool is
    /// registered or the ToolRegistry is updated) so the next system
    /// prompt build regenerates the tools listing.
    pub fn invalidate_tools(&mut self) {
        self.invalidate("tools");
    }

    /// Invalidate the skill listing section cache.
    ///
    /// Call this when skill files change so the next system prompt build
    /// regenerates the listing from the current registry state.
    pub fn invalidate_skill_listing(&mut self) {
        self.invalidate("skill_listing");
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

/// Load and cache a static file-based section.
/// Returns cached value if mtime unchanged; otherwise reloads and caches.
pub fn load_cached_file_section(
    cache: &mut SectionCache,
    name: &str,
    path: &Path,
) -> Option<String> {
    let (content, mtime) = read_file_section(path)?;

    if let Some(cached) = cache.get(name, Some(mtime)) {
        return Some(cached);
    }

    // Cache miss or stale — store and return
    cache.put(name, content.clone(), Some(mtime));
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
