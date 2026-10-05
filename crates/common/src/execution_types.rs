//! Execution-related pure data types shared across crates.
//!
//! Contains `ExecutionStepStatus`
//! which is consumed by the execution engine and session (recovery/persistence).

use serde::{Deserialize, Serialize};

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
