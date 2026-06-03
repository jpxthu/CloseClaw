//! System prompt dynamic-layer injection helpers.
//!
//! Helper functions for building dynamic sections and composing the full
//! system prompt.

use crate::gateway::session_handler::MessageMetadata;
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
    turn_count: u32,
    meta: &MessageMetadata,
    workdir_path: Option<&str>,
    system_appends: &[String],
) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();

    sections.push(Section::ChannelContext {
        chat_name: meta.channel.clone(),
        sender_id: meta.sender_id.clone(),
        timestamp: chrono::DateTime::from_timestamp(meta.timestamp, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
    });

    sections.push(Section::SessionState {
        turn_count,
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

/// Compose a full system prompt from static layer + dynamic sections.
///
/// Inserts `<!-- STATIC_LAYER_END -->` between static and dynamic layers.
pub(crate) fn build_full_system_prompt(
    static_prompt: Option<&str>,
    dynamic_sections: &[Section],
) -> String {
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
