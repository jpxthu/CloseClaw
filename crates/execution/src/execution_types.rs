//! Execution-specific types.
//!
//! `ExecutionStep` is defined here — the execution engine's own step payload.
//! The companion `ExecutionStepStatus` stays in `closeclaw-common::execution_types`
//! because session recovery/persistence consumes it as well.

use closeclaw_common::ExecutionStepStatus;
use serde::{Deserialize, Serialize};

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
