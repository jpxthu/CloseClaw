//! Plan Mode state types — shared across session and mode modules.
//!
//! `PlanState` is the minimal state structure for plan mode.
//! Execution step state machine methods have been migrated to
//! `closeclaw_execution::plan_state` as free functions.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Plan Path — plan 双路径选择
///
/// 标准路径（需求明确）或 Interview 路径（需求模糊）。
/// 无显式指定时由系统自动判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanPath {
    /// 标准路径：需求明确，4 阶段工作流
    #[default]
    Standard,
    /// Interview 路径：需求模糊，循环探索
    Interview,
}

impl std::fmt::Display for PlanPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Standard => write!(f, "standard"),
            Self::Interview => write!(f, "interview"),
        }
    }
}

/// Plan Phase — 当前规划阶段枚举
///
/// 阶段切换由 agent 自行判断，代码层不强制状态机转换。
/// 只存储 phase 值，不做行为绑定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanPhase {
    /// 研究阶段
    #[default]
    Research,
    /// 设计阶段
    Design,
    /// 审查阶段
    Review,
    /// 最终计划阶段
    FinalPlan,
    /// 访谈阶段
    Interview,
}

/// 执行步骤状态枚举
///
/// 状态机：pending → in_progress → completed | failed，
/// completed 不可回退，failed → in_progress 允许重试。
///
/// 已迁移到 `closeclaw_execution::plan_state`。此处保留以维持
/// serde 向后兼容性（PlanState 字段类型引用）。
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
///
/// 已迁移到 `closeclaw_execution::plan_state`。此处保留以维持
/// serde 向后兼容性（PlanState 字段类型引用）。
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

/// Plan Mode 状态 — 管理规划阶段、待办步骤和 plan 文件路径
///
/// 由 mode 模块创建，Session 持久化，Compaction 隔离保护，
/// Session 恢复时从 checkpoint 重建。
///
/// 执行步骤的完成状态由 Agent 写在 plan 文件中管理，系统不介入
/// 进度判断。执行步骤状态机相关的字段和方法已迁移到
/// `closeclaw_execution::plan_state` 模块。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlanState {
    /// 当前规划阶段
    #[serde(default)]
    pub phase: PlanPhase,
    /// 未完成的规划步骤标识列表
    #[serde(default)]
    pub pending_steps: Vec<String>,
    /// plan 文件路径 — Agent 写入和读取的唯一可写目标
    #[serde(default)]
    pub plan_file_path: String,
    /// 执行步骤列表（内部字段，供执行引擎使用）
    #[serde(default)]
    pub execution_steps: Vec<ExecutionStep>,
    /// 当前正在执行的步骤索引（内部字段，供执行引擎使用）
    #[serde(default)]
    pub current_step: Option<usize>,
    /// 显式指定的 plan 路径（None 表示由系统自动判断）
    #[serde(default)]
    pub explicit_path: Option<PlanPath>,
    /// Optional step selection (0-based indices) for partial execution.
    /// `None` means execute all steps; `Some(indices)` means execute
    /// only the specified steps.
    #[serde(default)]
    pub step_selection: Option<Vec<usize>>,
}

impl PlanState {
    /// 创建新的 PlanState，使用默认值（Research 阶段、空步骤、空路径）
    pub fn new() -> Self {
        Self::default()
    }
}

/// 步骤状态转换错误类型
///
/// 已迁移到 `closeclaw_execution::plan_state`。此处保留以维持
/// 跨 crate 引用兼容性。
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
