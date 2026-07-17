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
use closeclaw_common::{
    DynamicPromptBuilder, DynamicPromptContext, ModeTransition, PlanPath, SessionMode,
};
use closeclaw_gateway::session_handler::MessageMetadata;

/// Parameters for [`build_dynamic_sections`].
///
/// Bundles all per-request state needed to construct dynamic system prompt
/// sections (ChannelContext, SessionState, ModeInstruction, etc.).
pub struct DynamicSectionsParams<'a> {
    /// Inbound message metadata (sender, channel, timestamp).
    pub meta: &'a MessageMetadata,
    /// When `Some`, injects a `WorkingDirectory` section and builds git
    /// status for that path.
    pub workdir_path: Option<&'a str>,
    /// Per-session append list (`/system` subcommand).
    pub system_appends: &'a [String],
    /// Session creation timestamp override for ChannelContext.
    pub session_timestamp: Option<i64>,
    /// Current session mode (Normal / Plan / Auto).
    pub session_mode: SessionMode,
    /// Explicit plan path for Plan Mode (overrides auto-analysis).
    pub explicit_plan_path: Option<PlanPath>,
    /// User input text for automatic plan-path analysis.
    pub user_input: Option<&'a str>,
    /// One-shot mode transition to inject (should be `take`'d by caller).
    pub pending_mode_transition: Option<ModeTransition>,
}

/// Build dynamic sections from metadata and session state.
///
/// Constructs ChannelContext, SessionState, and optionally WorkingDirectory,
/// GitStatus, and AppendSection based on the provided parameters.
pub fn build_dynamic_sections(params: &DynamicSectionsParams<'_>) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();

    // Inject mode-specific instructions when not in Normal mode.
    if params.session_mode != SessionMode::Normal {
        // In Plan Mode, resolve the path: explicit override or auto-analysis.
        let resolved_plan_path = if params.session_mode == SessionMode::Plan {
            Some(
                params
                    .explicit_plan_path
                    .unwrap_or_else(|| analyze_plan_path(params.user_input.unwrap_or(""))),
            )
        } else {
            None
        };

        sections.push(Section::ModeInstruction {
            mode: params.session_mode,
            plan_path: resolved_plan_path,
            sparse: false,
            sub_agent: false,
        });
    }

    // Inject one-shot mode transition notification.
    // The value was already `take`'d by the session layer, so this is
    // a one-shot injection: the section appears only in the prompt
    // build immediately following the transition.
    if let Some(transition) = params.pending_mode_transition {
        sections.push(Section::ModeTransition { transition });
    }

    sections.push(Section::ChannelContext {
        chat_name: params.meta.channel.clone(),
        sender_id: params.meta.sender_id.clone(),
        timestamp: params
            .session_timestamp
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .or_else(|| chrono::DateTime::from_timestamp(params.meta.timestamp, 0))
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
    });

    sections.push(Section::SessionState {
        pending_tasks: vec![],
    });

    if let Some(path) = params.workdir_path {
        sections.push(Section::WorkingDirectory(path.to_string()));

        if let Some(status) = workdir::build_git_status_for(path) {
            sections.push(Section::GitStatus(status));
        }
    }

    if !params.system_appends.is_empty() {
        let body: String = params
            .system_appends
            .iter()
            .enumerate()
            .map(|(idx, content)| format!("[{}] {}", idx, content))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(Section::AppendSection(body));
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

/// Compose a full system prompt from static layer + dynamic sections.
///
/// Inserts `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` between static and dynamic layers.
///
/// When `overrides` is provided and contains a non-None priority prompt,
/// the resolution order is:
///   1. `override_prompt` — highest priority
///   2. `agent_prompt`    — agent-level prompt
///   3. `custom_prompt`   — user-defined custom prompt
///
/// On a priority hit the matched prompt **replaces** the static layer and
/// dynamic layers (ChannelContext / SessionState / GitStatus) are **not**
/// injected — only `AppendSection` entries are appended.
pub fn build_full_system_prompt(
    static_prompt: Option<&str>,
    dynamic_sections: &[Section],
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
            // Filter AppendSection from dynamic sections to append separately
            let append_parts: Vec<&str> = dynamic_sections
                .iter()
                .filter_map(|s| match s {
                    Section::AppendSection(body) => Some(body.as_str()),
                    _ => None,
                })
                .collect();

            if append_parts.is_empty() {
                return base.to_string();
            }
            let append_body = append_parts.join("\n");
            return format!("{}\n\n## Append\n{}\n", base, append_body);
        }
    }

    // Normal path: static + all dynamic sections
    let dynamic_rendered: String = dynamic_sections.iter().map(|s| s.render()).collect();
    if let Some(static_prompt) = static_prompt {
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
    }
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
        };

        // Check for priority prompt overrides (override > agent > custom).
        if let Some(ov) = context.overrides {
            let priority = ov
                .override_prompt
                .as_deref()
                .or(ov.agent_prompt.as_deref())
                .or(ov.custom_prompt.as_deref());

            if let Some(base) = priority {
                // Override replaces the static layer; only AppendSection
                // entries from the dynamic side are preserved.
                let sections = build_dynamic_sections(&DynamicSectionsParams {
                    meta: &meta,
                    workdir_path: None,
                    system_appends: context.system_appends,
                    session_timestamp: None,
                    session_mode: context.session_mode,
                    explicit_plan_path: None,
                    user_input: context.user_input,
                    pending_mode_transition: context.pending_mode_transition,
                });
                let append_parts: Vec<&str> = sections
                    .iter()
                    .filter_map(|s| match s {
                        Section::AppendSection(body) => Some(body.as_str()),
                        _ => None,
                    })
                    .collect();
                let dynamic = if append_parts.is_empty() {
                    None
                } else {
                    Some(append_parts.join("\n"))
                };
                return (Some(base.to_string()), dynamic);
            }
        }

        // Normal path: static layer from stored prompt, dynamic layer
        // freshly built from request context.
        let workdir_str = context.workdir.to_str().map(|s| s.to_owned());
        let sections = build_dynamic_sections(&DynamicSectionsParams {
            meta: &meta,
            workdir_path: workdir_str.as_deref(),
            system_appends: context.system_appends,
            session_timestamp: None, // use meta.timestamp, not session created_at
            session_mode: context.session_mode,
            explicit_plan_path: None,
            user_input: context.user_input,
            pending_mode_transition: context.pending_mode_transition,
        });
        let dynamic_rendered = if sections.is_empty() {
            None
        } else {
            Some(sections.iter().map(|s| s.render()).collect())
        };
        (
            context.system_prompt.map(|s| s.to_string()),
            dynamic_rendered,
        )
    }
}
