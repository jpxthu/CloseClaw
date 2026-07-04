//! Plan Mode state types — shared across session and mode modules.

use serde::{Deserialize, Serialize};

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
