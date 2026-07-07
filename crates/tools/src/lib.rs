//! Tools —两级工具体系核心类型
//!
//! # 架构
//! - [`Tool`] trait：定义工具核心接口（name / group / summary / detail / input_schema / flags）
//! - [`ToolFlags`]：bitflags 风格标记（is_concurrency_safe / is_read_only / is_destructive / etc.）
//! - [`ToolContext`]：运行时上下文（agent_id + workdir）
//! - [`ToolSummary`]：一级摘要数据，仅用于 system prompt 索引
//! - [`ToolError`]：工具层错误类型
//!
//! Tool trait 是新类型体系，与 [`crate::skills::Skill`] trait 完全独立：
//! - Skill = 领域知识按需读取
//! - Tool = LLM 可调用能力

pub mod builtin;
pub mod registrar;
pub mod registrars;
pub mod registry;
pub mod security;
pub mod spawn_validation;
pub mod tool_types;
pub mod workdir_context;

pub use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};
pub use registrars::core::CoreToolsRegistrar;
pub use registrars::plan::PlanToolsRegistrar;
pub use registrars::session::SessionToolsRegistrar;
pub use registrars::skills::SkillsToolsRegistrar;
pub use registry::ToolRegistryImpl;
pub use tool_types::{ToolError, ToolSummary};
/// Type alias for backward compatibility.
pub type ToolRegistry = ToolRegistryImpl;
pub use spawn_validation::{SpawnError, SpawnValidationResult, SpawnValidator};

// Re-export WorkdirContext helpers from common (via workdir_context module).
pub use workdir_context::{build_git_status_for, build_workdir_context, WorkdirContext};

// Re-export tool trait types from common (except ToolSummary/ToolError which live here,
// and build_git_status_for/build_workdir_context/WorkdirContext which come from workdir_context).
pub use closeclaw_common::tool_trait::{
    ContextModifier, PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolFlags,
    ToolMessage, ToolResult,
};
