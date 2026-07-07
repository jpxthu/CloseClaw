//! Sub-agent result parsing — extract structured results from
//! sub-agent notification text.
//!
//! Supports two formats:
//! - **JSON**: Full structured payload.
//! - **Tag format**: `[STEP:0][STATUS:completed][SUMMARY:...]` markers.

use closeclaw_common::ExecutionStepStatus;
use serde::Deserialize;

use crate::types::SubAgentResult;

/// Errors that occur when parsing a sub-agent result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Descriptive error message explaining why parsing failed.
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "parse error: {}", self.message)
    }
}

impl std::error::Error for ParseError {}

/// JSON schema for sub-agent result payloads.
#[derive(Debug, Deserialize)]
struct JsonPayload {
    step_index: Option<usize>,
    status: Option<String>,
    summary: Option<String>,
    changed_files: Option<Vec<String>>,
    error_message: Option<String>,
}

/// Parsed tag components from the marker format.
#[derive(Debug, Default)]
struct TagComponents {
    step_index: Option<usize>,
    status: Option<String>,
    summary: Option<String>,
    changed_files: Option<Vec<String>>,
    error_message: Option<String>,
}

/// Parse a sub-agent result from raw notification text.
///
/// Tries JSON first, then tag format. Returns a failed result with
/// error info if parsing fails entirely.
pub fn parse_subagent_result(raw: &str) -> Result<SubAgentResult, ParseError> {
    let trimmed = raw.trim();

    if trimmed.starts_with('{') {
        return parse_json(trimmed);
    }

    if trimmed.contains("[STEP:") || trimmed.contains("[STATUS:") {
        return parse_tags(trimmed);
    }

    Err(ParseError {
        message: "unrecognized format: not JSON and no [STEP:] markers found".to_string(),
    })
}

fn parse_json(raw: &str) -> Result<SubAgentResult, ParseError> {
    let payload: JsonPayload = serde_json::from_str(raw).map_err(|e| ParseError {
        message: format!("JSON parse failed: {e}"),
    })?;

    let step_index = payload.step_index.unwrap_or(0);
    let status = payload
        .status
        .and_then(|s| parse_status(&s))
        .unwrap_or(ExecutionStepStatus::Pending);
    let summary = payload.summary.unwrap_or_default();
    let changed_files = payload.changed_files.unwrap_or_default();
    let error_message = payload.error_message;

    Ok(SubAgentResult {
        step_index,
        status,
        summary,
        changed_files,
        error_message,
    })
}

fn parse_tags(raw: &str) -> Result<SubAgentResult, ParseError> {
    let mut components = TagComponents::default();

    for tag_content in extract_tag_contents(raw) {
        let (key, value) = tag_content.split_once(':').ok_or_else(|| ParseError {
            message: format!("malformed tag: [{tag_content}]"),
        })?;

        match key {
            "STEP" => {
                let idx = value.parse::<usize>().map_err(|e| ParseError {
                    message: format!("invalid STEP index '{value}': {e}"),
                })?;
                components.step_index = Some(idx);
            }
            "STATUS" => {
                components.status = Some(value.to_string());
            }
            "SUMMARY" => {
                components.summary = Some(value.to_string());
            }
            "FILES" => {
                let files: Vec<String> = value
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                components.changed_files = Some(files);
            }
            "ERROR" => {
                components.error_message = Some(value.to_string());
            }
            _ => {}
        }
    }

    let step_index = components.step_index.unwrap_or(0);
    let status = components
        .status
        .and_then(|s| parse_status(&s))
        .unwrap_or(ExecutionStepStatus::Pending);

    Ok(SubAgentResult {
        step_index,
        status,
        summary: components.summary.unwrap_or_default(),
        changed_files: components.changed_files.unwrap_or_default(),
        error_message: components.error_message,
    })
}

fn extract_tag_contents(raw: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = None;

    for (i, ch) in raw.char_indices() {
        if ch == '[' {
            start = Some(i + 1);
        } else if ch == ']' {
            if let Some(s) = start {
                result.push(raw[s..i].to_string());
                start = None;
            }
        }
    }

    result
}

fn parse_status(s: &str) -> Option<ExecutionStepStatus> {
    match s {
        "pending" => Some(ExecutionStepStatus::Pending),
        "in_progress" => Some(ExecutionStepStatus::InProgress),
        "completed" => Some(ExecutionStepStatus::Completed),
        "failed" => Some(ExecutionStepStatus::Failed),
        "skipped" => Some(ExecutionStepStatus::Skipped),
        _ => None,
    }
}
