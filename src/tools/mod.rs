//! Tools —两级工具体系核心类型
//!
//! # 架构
//! - [`Tool`] trait：定义工具核心接口（name / group / summary / detail / input_schema / flags）
//! - [`ToolFlags`]：bitflags 风格标记（is_concurrency_safe / is_read_only / is_destructive / etc.）
//! - [`ToolContext`]：运行时上下文（agent_id + workdir）
//! - [`ToolDescriptor`]：一级摘要数据，仅用于 system prompt 索引
//! - [`ToolError`]：工具层错误类型
//!
//! Tool trait 是新类型体系，与 [`crate::skills::Skill`] trait 完全独立：
//! - Skill = 领域知识按需读取
//! - Tool = LLM 可调用能力

pub mod builtin;
pub mod registry;

pub use registry::ToolRegistry;

use serde_json::Value;
use thiserror::Error;

// ---------------------------------------------------------------------------
// ToolFlags — bitflags 风格
// ---------------------------------------------------------------------------

/// Tool-level runtime flags.
///
/// All flags default to `false` unless noted.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolFlags {
    /// Tool is safe to call concurrently from multiple agents.
    pub is_concurrency_safe: bool,
    /// Tool only reads data, never modifies files or state.
    pub is_read_only: bool,
    /// Tool may overwrite or delete data — requires explicit confirmation.
    pub is_destructive: bool,
    /// Tool may be slow or consume significant resources.
    pub is_expensive: bool,
    /// Tool detail is NOT loaded into system prompt by default
    /// (requires explicit ToolSearch trigger).
    pub is_deferred_by_default: bool,
}

impl ToolFlags {
    /// Returns true if the tool should be loaded into the system prompt
    /// by default (i.e., NOT deferred).
    #[inline]
    pub fn is_eager(&self) -> bool {
        !self.is_deferred_by_default
    }
}

// ---------------------------------------------------------------------------
// ToolContext —运行时上下文
// ---------------------------------------------------------------------------

/// Runtime context passed to tools at call time.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// ID of the agent invoking this tool.
    pub agent_id: String,
    /// Current working directory context (if set).
    pub workdir: Option<crate::system_prompt::WorkdirContext>,
}

// ---------------------------------------------------------------------------
// ToolDescriptor —一级摘要数据
// ---------------------------------------------------------------------------

/// Reduced tool info for the system prompt index.
///
/// Contains only the fields needed to render the first-level
/// tool listing (group name + tool name + summary).
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
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
// ToolError —工具层错误
// ---------------------------------------------------------------------------

/// Errors raised by the tools layer.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool not found: {0}")]
    NotFound(String),

    #[error("tool `{0}` already registered")]
    AlreadyRegistered(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Tool —核心 trait
// ---------------------------------------------------------------------------

/// Core interface for a callable tool.
///
/// Each tool is a named, grouped capability that the LLM can invoke.
/// Implementations must be `Send + Sync + 'static`.
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

    /// Returns the JSON Schema for this tool's input parameters.
    fn input_schema(&self) -> Value;

    /// Returns this tool's runtime flags.
    fn flags(&self) -> ToolFlags;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_flags_default() {
        let flags = ToolFlags::default();
        assert!(!flags.is_concurrency_safe);
        assert!(!flags.is_read_only);
        assert!(!flags.is_destructive);
        assert!(!flags.is_expensive);
        assert!(!flags.is_deferred_by_default);
    }

    #[test]
    fn test_tool_flags_is_eager() {
        let mut flags = ToolFlags::default();
        assert!(flags.is_eager());

        flags.is_deferred_by_default = true;
        assert!(!flags.is_eager());
    }

    #[test]
    fn test_tool_descriptor_construction() {
        let desc = ToolDescriptor {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary: "Read file contents".to_string(),
            is_deferred: false,
        };
        assert_eq!(desc.name, "Read");
        assert_eq!(desc.group, "file_ops");
        assert!(!desc.is_deferred);
    }

    #[test]
    fn test_tool_error_display() {
        let err = ToolError::NotFound("Read".to_string());
        assert!(err.to_string().contains("Read"));
        assert!(err.to_string().contains("not found"));
    }
}
