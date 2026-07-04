//! ToolSummary and ToolError — tool-layer types.
//!
//! [`ToolSummary`] is the reduced tool info used for the system-prompt
//! index; [`ToolError`] covers tool-registry errors.

use thiserror::Error;

// ---------------------------------------------------------------------------
// ToolSummary — reduced tool info for the system prompt index
// ---------------------------------------------------------------------------

/// Reduced tool info for the system prompt index.
///
/// Contains only the fields needed to render the first-level
/// tool listing (group name + tool name + summary).
///
/// Named `ToolSummary` to avoid collision with
/// [`crate::registry::ToolRegistryImpl`] which carries richer
/// info for `ToolRegistryQuery`.
#[derive(Debug, Clone)]
pub struct ToolSummary {
    /// Unique tool name.
    pub name: String,
    /// Group this tool belongs to.
    pub group: String,
    /// Short one-line summary (≤50 chars).
    pub summary: String,
    /// Whether this tool's detail is deferred by default.
    pub is_deferred: bool,
}

// ---------------------------------------------------------------------------
// ToolError — tool layer errors
// ---------------------------------------------------------------------------

/// Errors raised by the tools layer.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool not found: {0}")]
    NotFound(String),

    #[error("tool `{0}` already registered")]
    AlreadyRegistered(String),

    #[error("tool registry is frozen — no further registrations accepted")]
    Frozen,

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
#[path = "tool_types_tests.rs"]
mod tests;
