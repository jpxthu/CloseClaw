//! Truncation logic for the Read tool.
//!
//! Implements three-threshold truncation (token, byte, line) and
//! generates continuation prompts that guide the agent to resume
//! reading from the correct offset.

/// Default maximum number of lines returned per Read call.
pub(crate) const DEFAULT_MAX_LINES: usize = 2000;

/// Default maximum byte size (50 KB) returned per Read call.
pub(crate) const DEFAULT_MAX_BYTES: usize = 51_200;

/// Approximate characters per token used for the token threshold.
const CHARS_PER_TOKEN: usize = 4;

/// Maximum byte size for a single line before triggering a special hint.
pub(crate) const SINGLE_LINE_BYTE_LIMIT: usize = 51_200;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for the truncation thresholds.
#[derive(Debug, Clone)]
pub struct TruncationConfig {
    /// Maximum number of lines to return.
    pub max_lines: usize,
    /// Maximum byte size to return.
    pub max_bytes: usize,
    /// Maximum approximate token count.
    pub max_tokens: usize,
}

impl Default for TruncationConfig {
    fn default() -> Self {
        Self {
            max_lines: DEFAULT_MAX_LINES,
            max_bytes: DEFAULT_MAX_BYTES,
            max_tokens: DEFAULT_MAX_BYTES / CHARS_PER_TOKEN,
        }
    }
}

/// Which threshold triggered the truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncationTrigger {
    /// Token limit was reached.
    Tokens,
    /// Byte limit was reached.
    Bytes,
    /// Line count limit was reached.
    Lines,
    /// User-specified limit was reached.
    Limit,
}

/// Result of a truncation operation.
#[derive(Debug, Clone)]
pub struct TruncationResult {
    /// The truncated content string.
    pub content: String,
    /// Whether the content was truncated (false means full file was returned).
    pub truncated: bool,
    /// Number of lines actually read and included in the output.
    pub lines_read: usize,
    /// Total number of lines in the original content.
    pub total_lines: usize,
    /// Which threshold triggered the truncation, if any.
    pub trigger: Option<TruncationTrigger>,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Truncate file content from `offset` (1-indexed) with optional `limit`.
///
/// Reads line-by-line from the given offset, accumulating bytes and
/// checking three thresholds (token → byte → line).  Also handles the
/// edge case where a single line exceeds `SINGLE_LINE_BYTE_LIMIT`.
pub(crate) fn truncate_lines(
    content: &str,
    offset: usize,
    limit: Option<usize>,
    config: &TruncationConfig,
) -> TruncationResult {
    let all_lines: Vec<&str> = content.lines().collect();
    let total_lines = all_lines.len();

    // offset is 1-indexed; clamp to valid range.
    let start = offset.saturating_sub(1).min(total_lines);

    // Empty or fully consumed content.
    if start >= total_lines || (limit == Some(0)) {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            lines_read: 0,
            total_lines,
            trigger: None,
        };
    }

    let lines = &all_lines[start..];

    // Special case: first line exceeds byte limit.
    let first_line_bytes = lines[0].len();
    if first_line_bytes > SINGLE_LINE_BYTE_LIMIT {
        let mut result = TruncationResult {
            content: lines[0].to_string(),
            truncated: true,
            lines_read: 1,
            total_lines,
            trigger: Some(TruncationTrigger::Bytes),
        };
        result.content.push('\n');
        return result;
    }

    // Accumulate lines, checking thresholds after each line.
    let mut accumulated_bytes: usize = 0;
    let mut accumulated_tokens: usize = 0;
    let mut line_count: usize = 0;
    let mut trigger: Option<TruncationTrigger> = None;

    for line in lines.iter() {
        // Enforce user-specified limit first.
        if let Some(max) = limit {
            if line_count >= max {
                trigger = Some(TruncationTrigger::Limit);
                break;
            }
        }

        // Check line count threshold.
        if line_count >= config.max_lines {
            trigger = Some(TruncationTrigger::Lines);
            break;
        }

        let line_bytes = line.len();
        let line_tokens = line_bytes / CHARS_PER_TOKEN;

        // Check token threshold.
        if accumulated_tokens + line_tokens > config.max_tokens {
            trigger = Some(TruncationTrigger::Tokens);
            break;
        }

        // Check byte threshold.
        if accumulated_bytes + line_bytes > config.max_bytes {
            trigger = Some(TruncationTrigger::Bytes);
            break;
        }

        accumulated_bytes += line_bytes;
        accumulated_tokens += line_tokens;
        line_count += 1;
    }

    let truncated = trigger.is_some() || start + line_count < total_lines;
    let mut content = String::new();
    for line in lines.iter().take(line_count) {
        content.push_str(line);
        content.push('\n');
    }

    TruncationResult {
        content,
        truncated,
        lines_read: line_count,
        total_lines,
        trigger,
    }
}

/// Generate a continuation prompt for the agent based on the truncation
/// result.  Returns `None` when the full file was returned.
pub(crate) fn format_truncation_message(
    result: &TruncationResult,
    offset: usize,
) -> Option<String> {
    if !result.truncated {
        return None;
    }

    let start = offset;
    let end = start + result.lines_read - 1;
    let next_offset = start + result.lines_read;

    match result.trigger {
        Some(TruncationTrigger::Lines) => Some(format!(
            "[Showing lines {start}-{end} of {}. \
             Use offset={next_offset} to continue.]",
            result.total_lines
        )),
        Some(TruncationTrigger::Bytes) => Some(format!(
            "[Showing lines {start}-{end} of {} \
             ({} limit). Use offset={next_offset} to continue.]",
            result.total_lines,
            human_readable_bytes(DEFAULT_MAX_BYTES),
        )),
        Some(TruncationTrigger::Tokens) => Some(format!(
            "[Showing lines {start}-{end} of {} \
             (token limit). Use offset={next_offset} to continue.]",
            result.total_lines,
        )),
        Some(TruncationTrigger::Limit) => {
            let remaining = result.total_lines - end;
            Some(format!(
                "[{remaining} more lines in file. \
                 Use offset={next_offset} to continue.]"
            ))
        }
        None => None,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Format a byte count as a human-readable string (e.g. "50KB").
fn human_readable_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{}MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}
