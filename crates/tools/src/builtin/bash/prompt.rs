use crate::PromptGenerationContext;

/// "When to use" section for the Bash tool prompt.
pub(super) fn prompt_when_to_use() -> String {
    "Use Bash to execute shell commands, run scripts, compile code, \
     or run tests. Supports timeout control (default 120s, max 600s), \
     output truncation with head-preservation (threshold 30,000 chars), \
     and output persistence to disk when output exceeds threshold. \
     Commands exceeding 15s are auto-backgrounded. Background tasks \
     notify automatically on completion — do not poll. \
     Use run_in_background for commands expected to exceed 10 seconds."
        .to_string()
}

/// Append workdir-based path guidance to the prompt parts.
pub(super) fn prompt_add_workdir_guidance(
    context: &PromptGenerationContext,
    parts: &mut Vec<String>,
) {
    if let Some(ref wd) = context.workdir {
        parts.push(closeclaw_common::format_workdir_guidance(
            wd,
            "Commands run here by default unless the cwd parameter is specified.",
        ));
    }
}

/// Append permission-awareness text. Uses runtime `available_tool_names` when available.
pub(super) fn prompt_add_permission_status(
    context: &PromptGenerationContext,
    parts: &mut Vec<String>,
) {
    let has_bash_access = if !context.available_tool_names.is_empty() {
        context
            .available_tool_names
            .iter()
            .any(|t| t.eq_ignore_ascii_case("Bash"))
    } else {
        context.tools.as_ref().is_none_or(|tools| {
            tools
                .iter()
                .any(|t| t.eq_ignore_ascii_case("Bash") || t == "*")
        })
    };
    if has_bash_access {
        parts.push(
            "Bash is available for your use. Commands are subject to \
             permission checks — untrusted commands may be sandboxed \
             or routed to the approval flow."
                .to_string(),
        );
    } else {
        parts.push(
            "Bash is not available in your current tool set. \
             You do not have permission to execute shell commands."
                .to_string(),
        );
    }
}

/// "Usage principles" section for the Bash tool prompt.
pub(super) fn prompt_usage_principles() -> String {
    "Keep commands concise and focused. Prefer specific commands \
     over complex pipelines when possible. Avoid long-running \
     foreground commands — use run_in_background for tasks \
     expected to exceed 10 seconds."
        .to_string()
}

/// Append combination suggestions based on available tools.
pub(super) fn prompt_add_combination_suggestions(
    context: &PromptGenerationContext,
    parts: &mut Vec<String>,
) {
    let has_read = context
        .available_tool_names
        .iter()
        .any(|t| t == "Read" || t == "read");
    let has_write = context
        .available_tool_names
        .iter()
        .any(|t| t == "Write" || t == "write");
    if has_read || has_write {
        let mut suggestions = Vec::new();
        if has_read {
            suggestions.push("Read (read files before editing)");
        }
        if has_write {
            suggestions.push("Write (redirect output to files)");
        }
        parts.push(format!("Combine with: {}.", suggestions.join(", ")));
    }
}
