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
    /// 执行步骤列表
    #[serde(default)]
    pub execution_steps: Vec<ExecutionStep>,
    /// 当前正在执行的步骤索引
    #[serde(default)]
    pub current_step: Option<usize>,
}

impl PlanState {
    /// 创建新的 PlanState，使用默认值（Research 阶段、空步骤、空路径）
    pub fn new() -> Self {
        Self::default()
    }

    /// 根据步骤描述列表初始化执行步骤（全部 pending），
    /// 重置 current_step = None
    pub fn init_execution_steps(&mut self, steps: Vec<String>) {
        self.execution_steps = steps
            .into_iter()
            .enumerate()
            .map(|(i, s)| ExecutionStep {
                step_index: i,
                status: ExecutionStepStatus::Pending,
                summary: s,
                error_message: None,
            })
            .collect();
        self.current_step = None;
    }

    /// 获取指定步骤的状态
    pub fn get_step_status(&self, step_index: usize) -> Option<&ExecutionStepStatus> {
        self.execution_steps.get(step_index).map(|s| &s.status)
    }

    /// 获取当前步骤索引
    pub fn current_step_index(&self) -> Option<usize> {
        self.current_step
    }
}
