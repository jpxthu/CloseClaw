//! Core Tool trait and associated types.
//!
//! This module defines the [`Tool`] trait and all supporting types
//! (`ToolContext`, `ToolResult`, `ToolCallError`, etc.) that were
//! previously in `closeclaw-tools`.  They now live in `closeclaw-common`
//! so that both the `tools` crate and downstream consumers can
//! reference them without circular dependencies.
//!
//! [`ToolSummary`] and [`ToolError`] have moved to the `tools` crate
//! (`closeclaw_tools::tool_types`).
//!
//! [`ToolFlags`] is re-exported from [`crate::tool_registry`] —
//! the single canonical definition lives there.

use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use thiserror::Error;

use crate::media_store::MediaStoreAccess;
use crate::session_mode::SessionMode;
use crate::tool_session::ToolSession;

// Re-export so downstream code can use `closeclaw_common::ToolFlags`.
pub use crate::tool_registry::ToolFlags;

// ---------------------------------------------------------------------------
// WorkdirContext — working directory context for tools
// ---------------------------------------------------------------------------

/// Format a workdir guidance line from a [`WorkdirContext`].
///
/// Produces a line like:
///
/// ```text
/// Working directory: /path/to/dir on branch 'main' (2 uncommitted change(s)).
/// ```
///
/// The optional `suffix` text is appended after the workdir line. Use it to
/// provide tool-specific guidance (e.g. "Commands run here by default.").
pub fn format_workdir_guidance(wd: &WorkdirContext, suffix: &str) -> String {
    let git_info = if wd.has_git {
        format!(
            " on branch '{}' ({} uncommitted change(s))",
            wd.branch.as_deref().unwrap_or("unknown"),
            wd.recent_changes
        )
    } else {
        " (not a git repo)".to_string()
    };
    format!("Working directory: {}{}. {}", wd.path, git_info, suffix)
}

/// Workdir context returned by [`build_workdir_context`].
#[derive(Debug, Clone)]
pub struct WorkdirContext {
    /// Absolute path to the working directory.
    pub path: String,
    /// Whether this is a git repository.
    pub has_git: bool,
    /// Current git branch (if has_git).
    pub branch: Option<String>,
    /// Number of uncommitted changes (if has_git).
    pub recent_changes: usize,
}

// ---------------------------------------------------------------------------
// ToolContext — runtime context
// ---------------------------------------------------------------------------

/// Runtime context passed to tools at call time.
pub struct ToolContext {
    /// ID of the agent invoking this tool.
    pub agent_id: String,
    /// Current working directory context (if set).
    pub workdir: Option<WorkdirContext>,
    /// ID of the session that owns this tool invocation, if known.
    pub session_id: Option<String>,
    /// The LLM-side tool call identifier.
    pub call_id: Option<String>,
    /// Strong reference to the owning session, abstracted via [`ToolSession`].
    pub session: Option<std::sync::Arc<dyn ToolSession>>,
    /// Current session mode (Normal, Plan, Auto), if known.
    ///
    /// When `Some(SessionMode::Plan)`, tools like `sessions_spawn` can
    /// reject operations that are not allowed in Plan Mode (e.g. fork).
    pub session_mode: Option<SessionMode>,
    /// Manual backgrounding signal from the session.
    ///
    /// Tools that execute long-running foreground commands (e.g. `BashTool`)
    /// can await `signal.notified()` inside `tokio::select!` to react to
    /// user-initiated manual backgrounding requests.
    pub manual_background_signal: Option<Arc<tokio::sync::Notify>>,
    /// Optional media store for resolving `[type: key]` references.
    pub media_store: Option<Arc<dyn MediaStoreAccess>>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("agent_id", &self.agent_id)
            .field("workdir", &self.workdir)
            .field("session_id", &self.session_id)
            .field("call_id", &self.call_id)
            .field(
                "session",
                &self.session.as_ref().map(|_| "<dyn ToolSession>"),
            )
            .field("session_mode", &self.session_mode)
            .field(
                "manual_background_signal",
                &self.manual_background_signal.as_ref().map(|_| "<Notify>"),
            )
            .field(
                "media_store",
                &self.media_store.as_ref().map(|_| "<dyn MediaStoreAccess>"),
            )
            .finish()
    }
}

impl Clone for ToolContext {
    fn clone(&self) -> Self {
        Self {
            agent_id: self.agent_id.clone(),
            workdir: self.workdir.clone(),
            session_id: self.session_id.clone(),
            call_id: self.call_id.clone(),
            session: self.session.clone(),
            session_mode: self.session_mode,
            manual_background_signal: self.manual_background_signal.clone(),
            media_store: self.media_store.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// build_workdir_context & build_git_status_for — helpers
// ---------------------------------------------------------------------------

/// Build a git status string for the given path (`None` if not a git repo).
pub fn build_git_status_for(path: &str) -> Option<String> {
    let p = Path::new(path);

    if !is_git_repo(p) {
        return None;
    }

    let branch = get_git_branch(p)?;
    let changes = count_uncommitted_changes(p);

    let status_summary = if changes == 0 {
        "clean".to_string()
    } else {
        format!("{} uncommitted change(s)", changes)
    };

    Some(format!(
        "On branch {}\n  status: {}",
        branch, status_summary
    ))
}

/// Build a [`WorkdirContext`] for the given path.
///
/// Resolves relative paths against `cwd`.  Canonicalizes the result.
pub fn build_workdir_context(path: &str) -> WorkdirContext {
    let abs_path = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };

    let canonical = abs_path.canonicalize().unwrap_or(abs_path);
    let has_git = is_git_repo(&canonical);
    let branch = if has_git {
        get_git_branch(&canonical)
    } else {
        None
    };
    let recent_changes = if has_git {
        count_uncommitted_changes(&canonical)
    } else {
        0
    };

    WorkdirContext {
        path: canonical.to_string_lossy().to_string(),
        has_git,
        branch,
        recent_changes,
    }
}

fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists() || find_git_root(path).is_some()
}

fn find_git_root(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path.to_path_buf());
    while let Some(p) = current {
        if p.join(".git").exists() {
            return Some(p);
        }
        current = p.parent().map(|p| p.to_path_buf());
    }
    None
}

fn get_git_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn count_uncommitted_changes(path: &Path) -> usize {
    let staged = git_status_count(path, "--cached");
    let unstaged = git_status_count(path, "");
    let untracked = git_status_count(path, "--others");

    staged + unstaged + untracked
}

fn git_status_count(path: &Path, extra_arg: &str) -> usize {
    let args: Vec<&str> = if extra_arg.is_empty() {
        vec!["status", "--porcelain"]
    } else {
        vec!["status", "--porcelain", extra_arg]
    };

    let output = Command::new("git")
        .args(&args)
        .current_dir(path)
        .output()
        .ok();

    output
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .count()
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// PromptGenerationContext — dynamic prompt generation context
// ---------------------------------------------------------------------------

/// Runtime context for dynamic tool prompt generation.
///
/// Distinct from [`ToolContext`]: `ToolContext` carries execution-time
/// information used when the LLM *calls* a tool, while
/// `PromptGenerationContext` carries information used when the system
/// prompt *describes* a tool. The two contexts evolve independently.
///
/// This context enables the Prompt layer (§Prompt 层) described in the
/// design doc `dynamic-prompt-generation.md` to produce context-aware
/// tool descriptions.  Each field corresponds to a runtime dimension
/// that may influence how a tool should be described to the model:
///
/// - **Permission state**: `tools` / `disallowed_tools` — which tools
///   the agent is allowed to use (whitelist/blacklist).
/// - **Working directory**: `workdir` — enables path-aware guidance.
/// - **Agent identity**: `agent_role` / `agent_type` — enables
///   role-based and type-based description adaptation.

#[derive(Debug, Clone, Default)]
pub struct PromptGenerationContext {
    /// ID of the agent for which the prompt is being built.
    pub agent_id: String,
    /// Current working directory context (if set).
    pub workdir: Option<WorkdirContext>,
    /// Names of all tools currently available to the LLM.
    ///
    /// Only names are passed (not descriptors) to avoid coupling
    /// `PromptGenerationContext` back to [`ToolDescriptor`] and to
    /// keep the context cheap to construct.
    pub available_tool_names: Vec<String>,
    /// Agent-level tool whitelist from config.
    ///
    /// When `Some(["*"])` or `Some([])` (or `None`), all tools are
    /// allowed.  When `Some(vec!["Read", "Bash", ...])`, only the
    /// listed tools are visible to the agent.
    pub tools: Option<Vec<String>>,
    /// Agent-level tool blacklist from config.
    ///
    /// Tools named here are excluded from the agent's tool set,
    /// applied *after* the whitelist filter.  `None` means no
    /// blacklist.
    pub disallowed_tools: Option<Vec<String>>,
    /// Session mode for mode-aware tool filtering.
    ///
    /// When `Some(SessionMode::Plan)`, write-capable tools are
    /// filtered out of the visible tool list (except plan-related
    /// tools). This implements the "write tools invisible in Plan
    /// Mode" requirement from the design doc.
    pub session_mode: Option<SessionMode>,
    /// Agent role (human-readable name/purpose) from agent config.
    ///
    /// Used by the Prompt layer to tailor tool descriptions to the
    /// agent's role.  `None` means the role is unknown (no role-based
    /// adaptation applied).
    pub agent_role: Option<String>,
    /// Agent type (e.g. root agent vs spawned child) from agent config.
    ///
    /// Used by the Prompt layer to tailor tool descriptions to the
    /// agent's type.  `None` means the type is unknown (no type-based
    /// adaptation applied).
    pub agent_type: Option<String>,
}

// ---------------------------------------------------------------------------
// ToolCallError — tool execution errors
// ---------------------------------------------------------------------------

/// Errors raised during tool execution.
#[derive(Debug, Error, Clone)]
pub enum ToolCallError {
    #[error("skill not found: {0}")]
    NotFound(String),

    #[error("permission denied for skill: {0}")]
    PermissionDenied(String),

    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    #[error("call not implemented for this tool")]
    NotImplemented,
}

// ---------------------------------------------------------------------------
// ToolMessage — context injection message
// ---------------------------------------------------------------------------

/// A message to be injected into the agent context.
#[derive(Debug, Clone)]
pub struct ToolMessage {
    /// The content of the message.
    pub content: String,
    /// Whether this message is metadata (not from a real user).
    pub is_meta: bool,
}

// ---------------------------------------------------------------------------
// ContextModifier — post-execution context modifier
// ---------------------------------------------------------------------------

/// Modifies the agent session context after tool execution.
#[derive(Debug, Clone)]
pub struct ContextModifier {
    /// List of tools the agent is allowed to use.
    pub allowed_tools: Vec<String>,
}

// ---------------------------------------------------------------------------
// ToolResult — tool execution result
// ---------------------------------------------------------------------------

/// Result of a tool call.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// The structured result data (JSON value).
    pub data: Value,
    /// Messages to inject into the agent context.
    pub new_messages: Vec<ToolMessage>,
    /// Optional context modifier.
    pub context_modifier: Option<ContextModifier>,
}

// ---------------------------------------------------------------------------
// Tool — core trait
// ---------------------------------------------------------------------------

/// Core interface for a callable tool.
///
/// Each tool is a named, grouped capability that the LLM can invoke.
/// Implementations must be `Send + Sync + 'static`.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Returns the unique name of this tool.
    fn name(&self) -> &str;

    /// Returns the group name (e.g. "file_ops", "meta").
    fn group(&self) -> &str;

    /// Returns a short one-line summary (≤50 chars) for the system prompt.
    ///
    /// This string is embedded verbatim into the first-level tool listing.
    fn summary(&self) -> String;

    /// Returns the detailed description for this tool.
    ///
    /// Shown when the LLM requests second-level detail (via ToolSearch).
    fn detail(&self) -> String;

    /// Returns the prompt-layer description for this tool.
    ///
    /// Unlike [`Tool::detail`] (which is a static string), `generate_prompt`
    /// can adapt its output to runtime context: current permissions, the set
    /// of available tools, the working directory, etc.
    ///
    /// # Default behaviour
    ///
    /// The default implementation falls back to [`Tool::detail`].  This is
    /// intentional: `detail()` is the natural-language equivalent of the
    /// Schema layer (JSON Schema) and serves as a safe static fallback
    /// when a tool has not yet opted into a custom Prompt layer.
    /// Tools that do not override `generate_prompt` will produce identical
    /// output to `detail()`, preserving backward compatibility.
    ///
    /// # Overriding
    ///
    /// Tools that implement a custom Prompt layer should override this
    /// method and use the fields on [`PromptGenerationContext`] to
    /// generate context-aware descriptions.  See the design doc
    /// `dynamic-prompt-generation.md` for the five dimensions that
    /// Prompt-layer descriptions should cover:
    ///
    /// 1. When to use the tool
    /// 2. Usage principles
    /// 3. Combination suggestions
    /// 4. Permission awareness
    /// 5. Path guidance
    fn generate_prompt(&self, _context: &PromptGenerationContext) -> String {
        self.detail()
    }

    /// Returns the JSON Schema for this tool's input parameters.
    fn input_schema(&self) -> Value;

    /// Call the tool with the given arguments and runtime context.
    ///
    /// Default implementation returns `ToolCallError::NotImplemented`.
    async fn call(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        Err(ToolCallError::NotImplemented)
    }

    /// Returns this tool's runtime flags.
    fn flags(&self) -> ToolFlags;
}

// ---------------------------------------------------------------------------
// Blanket impl: Box<dyn Tool> delegates to the inner tool
// ---------------------------------------------------------------------------

#[async_trait]
impl Tool for Box<dyn Tool> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn group(&self) -> &str {
        (**self).group()
    }
    fn summary(&self) -> String {
        (**self).summary()
    }
    fn detail(&self) -> String {
        (**self).detail()
    }
    fn generate_prompt(&self, context: &PromptGenerationContext) -> String {
        (**self).generate_prompt(context)
    }
    fn input_schema(&self) -> Value {
        (**self).input_schema()
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        (**self).call(args, ctx).await
    }
    fn flags(&self) -> ToolFlags {
        (**self).flags()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that PromptGenerationContext does NOT contain `effective_spawn_budget`.
    /// This struct literal pattern will fail to compile if the field is re-added,
    /// enforcing the Step 1.2 removal.
    #[test]
    fn test_prompt_generation_context_no_effective_spawn_budget() {
        let ctx = PromptGenerationContext {
            agent_id: "test".into(),
            workdir: None,
            available_tool_names: vec![],
            tools: None,
            disallowed_tools: None,
            session_mode: None,
            agent_role: None,
            agent_type: None,
        };
        // Struct destructure — compile fails if field list changes.
        let PromptGenerationContext {
            agent_id: _,
            workdir: _,
            available_tool_names: _,
            tools: _,
            disallowed_tools: _,
            session_mode: _,
            agent_role: _,
            agent_type: _,
        } = ctx;
    }

    /// Verify that Default impl works without any budget-related fields.
    #[test]
    fn test_prompt_generation_context_default_no_budget() {
        let ctx = PromptGenerationContext::default();
        assert!(ctx.agent_id.is_empty());
        assert!(ctx.workdir.is_none());
        assert!(ctx.available_tool_names.is_empty());
        assert!(ctx.tools.is_none());
        assert!(ctx.disallowed_tools.is_none());
        assert!(ctx.session_mode.is_none());
        assert!(ctx.agent_role.is_none());
        assert!(ctx.agent_type.is_none());
    }
}
