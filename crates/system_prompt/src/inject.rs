//! System prompt dynamic-layer injection helpers.
//!
//! Helper functions for building dynamic sections and composing the full
//! system prompt.
//!
//! Migrated from `gateway::system_prompt_inject` — these functions logically
//! belong to the `system_prompt` module.

use crate::builder::PromptOverrides;
use crate::plan_path::analyze_plan_path;
use crate::sections::Section;
use crate::workdir;
use closeclaw_common::system_prompt::ModeTransition;
use closeclaw_common::{DynamicPromptBuilder, DynamicPromptContext, SessionMode};
use closeclaw_execution::PlanPath;
use closeclaw_gateway::session_handler::MessageMetadata;

/// Parameters for [`build_dynamic_sections`].
///
/// Bundles all per-request state needed to construct dynamic system prompt
/// sections (ChannelContext, WorkingDirectory, ModeInstruction, GitStatus).
pub struct DynamicSectionsParams<'a> {
    /// Inbound message metadata (sender, channel, timestamp).
    pub meta: &'a MessageMetadata,
    /// When `Some`, injects a `WorkingDirectory` section and builds git
    /// status for that path.
    pub workdir_path: Option<&'a str>,
    /// Current session mode (Normal / Plan / Auto).
    pub session_mode: SessionMode,
    /// Explicit plan path for Plan Mode (overrides auto-analysis).
    pub explicit_plan_path: Option<PlanPath>,
    /// User input text for automatic plan-path analysis.
    pub user_input: Option<&'a str>,
    /// Whether the session context has been compacted (for sparse prompt injection).
    pub is_compacted: bool,
    /// Whether this prompt is for a sub-agent (for sub-agent sparse injection).
    pub is_sub_agent: bool,
    /// When `true`, injects GitStatus section when workdir is a git repo.
    pub is_git_status_enabled: bool,
    /// Mode transition that triggered this prompt build.
    ///
    /// When `Some`, a `ModeTransition` section is injected with the
    /// corresponding design doc §6 prompt. `None` means no transition
    /// occurred on this request.
    pub mode_transition: Option<ModeTransition>,
}

/// Build dynamic sections from metadata and session state.
///
/// Constructs exactly the four dynamic sections defined by the design doc:
/// - **ChannelContext** (always)
/// - **WorkingDirectory** (when `workdir_path` is provided)
/// - **ModeInstruction** (when not Normal mode)
/// - **GitStatus** (when enabled and workdir is a git repo)
///
/// Appends (追加区) are NOT built here; they are assembled independently
/// in [`build_full_system_prompt`] or by the caller.
pub fn build_dynamic_sections(params: &DynamicSectionsParams<'_>) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();

    // 1. ChannelContext (always injected)
    sections.push(Section::ChannelContext {
        chat_name: params.meta.chat_name.clone(),
    });

    // 2. WorkingDirectory (when workdir_path is provided)
    if let Some(path) = params.workdir_path {
        sections.push(Section::WorkingDirectory(path.to_string()));
    }

    // 3. ModeInstruction (when not Normal mode)
    if params.session_mode != SessionMode::Normal {
        // In Plan Mode, resolve the path: explicit override or auto-analysis.
        // When no explicit path and no user input, return None to inject
        // path selection rules (design doc §1).
        let resolved_plan_path = if params.session_mode == SessionMode::Plan {
            if let Some(explicit) = params.explicit_plan_path {
                Some(explicit)
            } else if let Some(input) = params.user_input {
                Some(analyze_plan_path(input))
            } else {
                None
            }
        } else {
            None
        };

        sections.push(Section::ModeInstruction {
            mode: params.session_mode,
            plan_path: resolved_plan_path,
            sparse: params.is_compacted,
            sub_agent: params.is_sub_agent,
        });
    }

    // Mode transition prompt injection (design doc §6).
    // Injected when a session mode change occurred on this request.
    if let Some(transition) = params.mode_transition {
        sections.push(Section::ModeTransition(transition));
    }

    // 4. GitStatus (when enabled and workdir is a git repo)
    if let Some(path) = params.workdir_path {
        if params.is_git_status_enabled {
            if let Some(status) = workdir::build_git_status_for(path) {
                sections.push(Section::GitStatus(status));
            }
        }
    }

    sections
}

/// Split a full system prompt into static and dynamic parts.
///
/// Uses the `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` boundary marker as the split point:
///
/// - Content **before** the first marker → `Some(static)` (trailing whitespace trimmed)
/// - Content **after** the first marker → `Some(dynamic)` (leading whitespace trimmed)
/// - No marker → `(Some(full_prompt.to_owned()), None)`
/// - Empty string → `(None, None)`
pub fn split_static_dynamic(full_prompt: &str) -> (Option<String>, Option<String>) {
    if full_prompt.is_empty() {
        return (None, None);
    }

    let marker = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";
    match full_prompt.find(marker) {
        Some(pos) => {
            let static_part = full_prompt[..pos].trim_end().to_owned();
            let dynamic_part = full_prompt[pos + marker.len()..].trim_start().to_owned();

            let s = if static_part.is_empty() {
                None
            } else {
                Some(static_part)
            };
            let d = if dynamic_part.is_empty() {
                None
            } else {
                Some(dynamic_part)
            };
            (s, d)
        }
        None => (Some(full_prompt.to_owned()), None),
    }
}

/// Compose a full system prompt from static layer + dynamic sections + appends.
///
/// Inserts `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` between static and dynamic layers,
/// then appends the appends section at the end.
///
/// When `overrides` is provided and contains a non-None priority prompt,
/// the resolution order is:
///   1. `override_prompt` — highest priority
///   2. `agent_prompt`    — agent-level prompt
///   3. `custom_prompt`   — user-defined custom prompt
///
/// On a priority hit the matched prompt **replaces** the static layer and
/// dynamic layers (ChannelContext / GitStatus) are **not**
/// injected — only `appends` entries are appended at the end.
pub fn build_full_system_prompt(
    static_prompt: Option<&str>,
    dynamic_sections: &[Section],
    appends: &[String],
    overrides: Option<&PromptOverrides>,
) -> String {
    // Check priority prompt overrides (override > agent > custom)
    if let Some(ov) = overrides {
        let priority = ov
            .override_prompt
            .as_deref()
            .or(ov.agent_prompt.as_deref())
            .or(ov.custom_prompt.as_deref());

        if let Some(base) = priority {
            // Priority prompt replaces static + dynamic layers; only appends are appended.
            if appends.is_empty() {
                return base.to_string();
            }
            let append_body = render_appends(appends);
            return format!("{}\n\n## Append\n{}\n", base, append_body);
        }
    }

    // Normal path: static + dynamic sections + appends
    let dynamic_rendered: String = dynamic_sections.iter().map(|s| s.render()).collect();
    let append_rendered = render_appends(appends);
    let mut result = if let Some(static_prompt) = static_prompt {
        if dynamic_rendered.is_empty() {
            static_prompt.to_string()
        } else {
            format!(
                "{}\n__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__\n{}",
                static_prompt, dynamic_rendered
            )
        }
    } else {
        dynamic_rendered
    };
    if !append_rendered.is_empty() {
        result.push_str("\n\n## Append\n");
        result.push_str(&append_rendered);
        result.push('\n');
    }
    result
}

/// Format appends as a numbered list for the append section.
fn render_appends(appends: &[String]) -> String {
    appends
        .iter()
        .enumerate()
        .map(|(idx, content)| format!("[{}] {}", idx, content))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── DynamicPromptBuilder adapter ───────────────────────────────────────────

/// Adapter implementing [`DynamicPromptBuilder`] for the system_prompt crate.
///
/// Bridges the session-layer trait to the concrete
/// [`build_dynamic_sections`] / [`build_full_system_prompt`] functions.
pub struct SystemPromptDynamicBuilder;

impl DynamicPromptBuilder for SystemPromptDynamicBuilder {
    fn build_prompt_parts(
        &self,
        context: &DynamicPromptContext,
    ) -> (Option<String>, Option<String>) {
        let meta = MessageMetadata {
            sender_id: context.ctx.sender_id.clone(),
            channel: context.ctx.channel.clone(),
            timestamp: context.ctx.timestamp,
            chat_name: context.ctx.chat_name.clone(),
            trace_id: None,
            session_key: None,
        };

        // Check for priority prompt overrides (override > agent > custom).
        if let Some(ov) = context.overrides {
            let priority = ov
                .override_prompt
                .as_deref()
                .or(ov.agent_prompt.as_deref())
                .or(ov.custom_prompt.as_deref());

            if let Some(base) = priority {
                // Override replaces the static layer; only appends are preserved.
                if context.system_appends.is_empty() {
                    return (Some(base.to_string()), None);
                }
                let append_body = render_appends(context.system_appends);
                let dynamic = format!("\n\n## Append\n{}\n", append_body);
                return (Some(base.to_string()), Some(dynamic));
            }
        }

        // Normal path: static layer from stored prompt, dynamic layer
        // freshly built from request context.
        let workdir_str = context.workdir.to_str().map(|s| s.to_owned());
        let sections = build_dynamic_sections(&DynamicSectionsParams {
            meta: &meta,
            workdir_path: workdir_str.as_deref(),
            session_mode: context.session_mode,
            explicit_plan_path: None,
            user_input: context.user_input,
            is_compacted: context.is_compacted,
            is_sub_agent: context.is_sub_agent,
            is_git_status_enabled: context.is_git_status_enabled,
            mode_transition: context.mode_transition,
        });
        let mut dynamic_rendered: String = sections.iter().map(|s| s.render()).collect();
        // Append the appends section directly (independent of dynamic sections)
        if !context.system_appends.is_empty() {
            let append_body = render_appends(context.system_appends);
            dynamic_rendered.push_str("\n\n## Append\n");
            dynamic_rendered.push_str(&append_body);
            dynamic_rendered.push('\n');
        }
        let dynamic = if dynamic_rendered.is_empty() {
            None
        } else {
            Some(dynamic_rendered)
        };
        (context.system_prompt.map(|s| s.to_string()), dynamic)
    }
}
