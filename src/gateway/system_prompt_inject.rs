//! System prompt dynamic-layer injection helpers.
//!
//! Helper functions for building dynamic sections and composing the full
//! system prompt.

use crate::gateway::session_handler::MessageMetadata;
use crate::system_prompt::builder::PromptOverrides;
use crate::system_prompt::sections::Section;
use crate::system_prompt::workdir;

/// Build dynamic sections from metadata and session state.
///
/// Constructs ChannelContext, SessionState, and optionally WorkingDirectory,
/// GitStatus, and AppendSection.
///
/// `workdir_path` — when `Some`, injects a `WorkingDirectory` section and
/// builds git status for that path instead of using global state.
///
/// `system_appends` — per-session append list (managed by `/system`
/// subcommand). When non-empty, a single `AppendSection` is pushed at
/// the end of the section list, formatted as a `[N] 内容` numbered
/// list in insertion order.
pub(crate) fn build_dynamic_sections(
    meta: &MessageMetadata,
    workdir_path: Option<&str>,
    system_appends: &[String],
    session_timestamp: Option<i64>,
) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();

    sections.push(Section::ChannelContext {
        chat_name: meta.channel.clone(),
        sender_id: meta.sender_id.clone(),
        timestamp: session_timestamp
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .or_else(|| chrono::DateTime::from_timestamp(meta.timestamp, 0))
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
    });

    sections.push(Section::SessionState {
        pending_tasks: vec![],
    });

    if let Some(path) = workdir_path {
        sections.push(Section::WorkingDirectory(path.to_string()));

        if let Some(status) = workdir::build_git_status_for(path) {
            sections.push(Section::GitStatus(status));
        }
    }

    if !system_appends.is_empty() {
        let body: String = system_appends
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
/// Uses the `<!-- STATIC_LAYER_END -->` boundary marker as the split point:
///
/// - Content **before** the first marker → `Some(static)` (trailing whitespace trimmed)
/// - Content **after** the first marker → `Some(dynamic)` (leading whitespace trimmed)
/// - No marker → `(Some(full_prompt.to_owned()), None)`
/// - Empty string → `(None, None)`
pub(crate) fn split_static_dynamic(full_prompt: &str) -> (Option<String>, Option<String>) {
    if full_prompt.is_empty() {
        return (None, None);
    }

    let marker = "<!-- STATIC_LAYER_END -->";
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
/// Inserts `<!-- STATIC_LAYER_END -->` between static and dynamic layers.
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
pub(crate) fn build_full_system_prompt(
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
                "{}\n<!-- STATIC_LAYER_END -->\n{}",
                static_prompt, dynamic_rendered
            )
        }
    } else {
        dynamic_rendered
    }
}
