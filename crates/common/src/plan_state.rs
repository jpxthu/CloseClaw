//! Plan Mode state types — shared across session and mode modules.

use serde::{Deserialize, Serialize};
use thiserror::Error;

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

    /// 生成格式化的执行进度摘要
    ///
    /// 返回空字符串当无执行步骤时。
    /// 格式示例：
    /// ```text
    /// ## Execution Progress
    /// Step 1/3: completed (done)
    /// → Step 2/3: in_progress
    /// Step 3/3: pending
    /// ```
    pub fn progress_summary(&self) -> String {
        if self.execution_steps.is_empty() {
            return String::new();
        }
        let total = self.execution_steps.len();
        let mut lines = Vec::with_capacity(total + 1);
        lines.push("## Execution Progress".to_string());
        for step in &self.execution_steps {
            let idx = step.step_index + 1;
            let is_current = self.current_step == Some(step.step_index);
            let marker = if is_current { "→ " } else { "" };
            let status_str = match step.status {
                ExecutionStepStatus::Pending => "pending".to_string(),
                ExecutionStepStatus::InProgress => "in_progress".to_string(),
                ExecutionStepStatus::Completed => {
                    if step.summary.is_empty() {
                        "completed".to_string()
                    } else {
                        format!("completed ({})", step.summary)
                    }
                }
                ExecutionStepStatus::Failed => match &step.error_message {
                    Some(e) => format!("failed ({})", e),
                    None => "failed".to_string(),
                },
                ExecutionStepStatus::Skipped => "skipped".to_string(),
            };
            lines.push(format!("{marker}Step {idx}/{total}: {status_str}"));
        }
        lines.join("\n")
    }

    /// 校验步骤状态转换是否合法
    pub fn validate_transition(
        &self,
        step_index: usize,
        new_status: &ExecutionStepStatus,
    ) -> Result<(), TransitionError> {
        let steps_len = self.execution_steps.len();
        if step_index >= steps_len {
            return Err(TransitionError::OutOfBounds {
                index: step_index,
                len: steps_len,
            });
        }

        // Skip-step check: step_index must == current_step (if set) or == 0
        if let Some(cur) = self.current_step {
            if step_index != cur {
                return Err(TransitionError::SkippedStep {
                    expected: cur,
                    got: step_index,
                });
            }
        } else if step_index != 0 {
            return Err(TransitionError::SkippedStep {
                expected: 0,
                got: step_index,
            });
        }

        let current = &self.execution_steps[step_index].status;
        let valid = match new_status {
            ExecutionStepStatus::InProgress => {
                matches!(
                    current,
                    ExecutionStepStatus::Pending | ExecutionStepStatus::Failed
                )
            }
            ExecutionStepStatus::Completed => {
                matches!(current, ExecutionStepStatus::InProgress)
            }
            ExecutionStepStatus::Failed => {
                matches!(current, ExecutionStepStatus::InProgress)
            }
            ExecutionStepStatus::Skipped => {
                matches!(current, ExecutionStepStatus::Pending)
            }
            ExecutionStepStatus::Pending => false,
        };

        if valid {
            Ok(())
        } else {
            Err(TransitionError::InvalidTransition {
                from: *current,
                to: *new_status,
            })
        }
    }

    /// 执行步骤状态转换：校验后更新状态和 current_step
    pub fn apply_transition(
        &mut self,
        step_index: usize,
        new_status: ExecutionStepStatus,
    ) -> Result<(), TransitionError> {
        self.validate_transition(step_index, &new_status)?;
        self.execution_steps[step_index].status = new_status;

        // Update current_step based on new status
        if matches!(
            new_status,
            ExecutionStepStatus::Completed | ExecutionStepStatus::Skipped
        ) {
            let next = step_index + 1;
            if next < self.execution_steps.len() {
                self.current_step = Some(next);
            }
        }
        // Failed: keep current_step unchanged
        // InProgress: current_step stays at step_index (already set or will be by caller)

        Ok(())
    }
}

/// 步骤状态转换错误类型
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TransitionError {
    /// 步骤索引不存在
    #[error("step not found: index {index} out of range (len {len})")]
    OutOfBounds { index: usize, len: usize },

    /// 非法状态转换
    #[error("invalid transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: ExecutionStepStatus,
        to: ExecutionStepStatus,
    },

    /// 跳步：目标步骤索引必须是 current_step 或 0（首次）
    #[error("skipped step: expected {expected}, got {got}")]
    SkippedStep { expected: usize, got: usize },
}
