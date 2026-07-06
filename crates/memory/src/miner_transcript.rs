//! Transcript cleaning for the memory miner.
//!
//! Raw session transcripts contain thinking blocks, tool-call XML, internal
//! context markers, and other noise that must be stripped before passing to
//! the LLM extraction stage. This module provides [`clean_transcript`].

use closeclaw_config::agents::{
    default_transcript_format, default_transcript_min_owner_msgs, default_transcript_min_turns,
    TranscriptCleanRules,
};

/// Clean a raw session transcript for LLM extraction.
///
/// Strips thinking blocks, tool-call XML, internal context markers,
/// MEDIA/NO_REPLY lines, and collapses consecutive blank lines.
/// Format defaults to markdown (`md`).
pub fn clean_transcript(raw: &str, rules: &TranscriptCleanRules) -> String {
    let min_turns = rules.min_turns.unwrap_or_else(default_transcript_min_turns) as usize;
    let min_owner_msgs = rules
        .min_owner_msgs
        .unwrap_or_else(default_transcript_min_owner_msgs) as usize;

    let lines = count_lines(raw);
    if lines < min_turns {
        return String::new();
    }

    let owner_count = count_owner_messages(raw);
    if owner_count < min_owner_msgs {
        return String::new();
    }

    let cleaned = strip_thinking_blocks(raw);
    let cleaned = strip_tool_calls(&cleaned);
    let cleaned = strip_context_markers(&cleaned);
    let cleaned = strip_noise_lines(&cleaned);
    let cleaned = collapse_blank_lines(&cleaned);
    let format = rules
        .format
        .clone()
        .unwrap_or_else(default_transcript_format);

    format_output(&cleaned, &format)
}

/// Count non-empty lines (proxy for conversation turns).
fn count_lines(text: &str) -> usize {
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// Count lines starting with "Owner:" or "user:" (owner messages).
fn count_owner_messages(text: &str) -> usize {
    text.lines()
        .filter(|l| {
            let trimmed = l.trim();
            trimmed.starts_with("Owner:")
                || trimmed.starts_with("owner:")
                || trimmed.starts_with("user:")
        })
        .count()
}

/// Remove `<thinking>...</thinking>` blocks.
fn strip_thinking_blocks(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_thinking = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("<thinking>") {
            in_thinking = true;
            continue;
        }
        if trimmed.contains("</thinking>") {
            in_thinking = false;
            continue;
        }
        if !in_thinking {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

/// Remove lines containing tool-call XML tags.
fn strip_tool_calls(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let trimmed = l.trim();
            !trimmed.contains("<tool_call>")
                && !trimmed.contains("</tool_call>")
                && !trimmed.contains("<tool_result>")
                && !trimmed.contains("</tool_result>")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Remove internal context markers like `[context: ...]`.
fn strip_context_markers(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim().starts_with("[context:"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Remove MEDIA and NO_REPLY lines.
fn strip_noise_lines(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let trimmed = l.trim();
            trimmed != "MEDIA" && trimmed != "NO_REPLY"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Collapse consecutive blank lines into a single blank line.
fn collapse_blank_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_blank = false;

    for line in text.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && prev_blank {
            continue;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(line);
        prev_blank = is_blank;
    }
    result
}

/// Format the cleaned transcript according to the target format.
fn format_output(text: &str, format: &str) -> String {
    match format {
        "md" | "markdown" => text.trim().to_string(),
        "plain" | "text" => text.trim().to_string(),
        _ => text.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(min_turns: i32, min_owner: i32) -> TranscriptCleanRules {
        TranscriptCleanRules {
            min_turns: Some(min_turns),
            min_owner_msgs: Some(min_owner),
            format: Some("md".to_string()),
        }
    }

    #[test]
    fn test_clean_transcript_removes_thinking() {
        let raw = "Owner: hello\n<thinking>\nI think...\n</thinking>\nAgent: hi";
        let cleaned = clean_transcript(raw, &rules(1, 1));
        assert!(!cleaned.contains("thinking"));
        assert!(!cleaned.contains("I think"));
        assert!(cleaned.contains("hello"));
        assert!(cleaned.contains("hi"));
    }

    #[test]
    fn test_clean_transcript_removes_tool_calls() {
        let raw =
            "Owner: go\n<tool_call>exec</tool_call>\n<tool_result>ok</tool_result>\nAgent: done";
        let cleaned = clean_transcript(raw, &rules(1, 1));
        assert!(!cleaned.contains("tool_call"));
        assert!(!cleaned.contains("tool_result"));
        assert!(cleaned.contains("done"));
    }

    #[test]
    fn test_clean_transcript_removes_context_markers() {
        let raw = "Owner: test\n[context: system]\nAgent: response";
        let cleaned = clean_transcript(raw, &rules(1, 1));
        assert!(!cleaned.contains("[context:"));
    }

    #[test]
    fn test_clean_transcript_removes_noise_lines() {
        let raw = "Owner: msg\nMEDIA\nNO_REPLY\nAgent: ok";
        let cleaned = clean_transcript(raw, &rules(1, 1));
        assert!(!cleaned.contains("MEDIA"));
        assert!(!cleaned.contains("NO_REPLY"));
    }

    #[test]
    fn test_clean_transcript_collapses_blank_lines() {
        let raw = "Owner: hi\n\n\n\nAgent: bye";
        let cleaned = clean_transcript(raw, &rules(1, 1));
        assert!(!cleaned.contains("\n\n\n"));
    }

    #[test]
    fn test_clean_transcript_too_few_turns() {
        let raw = "Owner: hi";
        let cleaned = clean_transcript(raw, &rules(5, 5));
        assert!(cleaned.is_empty());
    }

    #[test]
    fn test_clean_transcript_too_few_owner_msgs() {
        let raw = "User: a\nAssistant: b\nUser: c\nAssistant: d\nUser: e\nAssistant: f";
        let cleaned = clean_transcript(raw, &rules(3, 10));
        assert!(cleaned.is_empty());
    }

    #[test]
    fn test_clean_transcript_normalizes_owner_prefix() {
        let raw = "Owner: a\nuser: b\nowner: c";
        let cleaned = clean_transcript(raw, &rules(1, 1));
        assert!(cleaned.contains("a"));
        assert!(cleaned.contains("b"));
        assert!(cleaned.contains("c"));
    }
}
