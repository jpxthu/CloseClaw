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
pub mod security;

pub use registry::ToolRegistry;

// Re-export all core types from closeclaw-tools crate to unify the Tool trait
// across the workspace. The main crate's builtin tools and external adapter
// tools (e.g. feishu) now share the same trait definition.
pub use closeclaw_tools::{
    ContextModifier, PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolDescriptor,
    ToolError, ToolFlags, ToolMessage, ToolResult,
};
