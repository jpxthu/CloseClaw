//! Plan Mode state types — shared across session and mode modules.
//!
//! `PlanState` is the minimal state structure for plan mode,
//! containing only `phase`, `pending_steps`, and `plan_file_path`.

use serde::{Deserialize, Serialize};

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

/// Plan Mode 状态 — 管理规划阶段、待办步骤和 plan 文件路径
///
/// 由 mode 模块创建，Session 持久化，Compaction 隔离保护，
/// Session 恢复时从 checkpoint 重建。
///
/// 执行步骤的完成状态由 Agent 写在 plan 文件中管理，系统不介入
/// 进度判断。执行步骤状态机相关的类型和方法在
/// `closeclaw_execution` crate 中。
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
}

impl PlanState {
    /// 创建新的 PlanState，使用默认值（Research 阶段、空步骤、空路径）
    pub fn new() -> Self {
        Self::default()
    }
}
