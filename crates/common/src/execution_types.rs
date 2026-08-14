//! Execution-related pure data types shared across crates.
//!
//! Contains `ExecutionStep`, `ExecutionStepStatus`, and `TransitionError`
//! which are consumed by the execution engine, tools (ProgressTool),
//! and session (recovery/persistence).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 执行步骤状态枚举
///
/// 状态机：pending → in_progress → completed | failed，
/// completed 不可回退，failed → in_progress 允许重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStepStatus {
    /// 待执行
    #[default]
    Pending,
    /// 执行中
    InProgress,
    /// 已完成
    Completed,
    /// 执行失败
    Failed,
    /// 已跳过
    Skipped,
}

/// 执行步骤 — 描述单个步骤的当前状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionStep {
    /// 步骤索引（从 0 开始）
    pub step_index: usize,
    /// 当前状态
    #[serde(default)]
    pub status: ExecutionStepStatus,
    /// 步骤描述或摘要
    #[serde(default)]
    pub summary: String,
    /// 失败时的错误信息
    #[serde(default)]
    pub error_message: Option<String>,
}

/// 步骤状态转换错误类型
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TransitionError {
    /// 步骤索引不存在
    #[error("step not found: index {index} out of range (len {len})")]
    OutOfBounds { index: usize, len: usize },

    /// 非法步骤状态转换
    #[error("invalid transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: ExecutionStepStatus,
        to: ExecutionStepStatus,
    },

    /// 跳步：目标步骤索引必须是 current_step 或 0（首次）
    #[error("skipped step: expected {expected}, got {got}")]
    SkippedStep { expected: usize, got: usize },
}
