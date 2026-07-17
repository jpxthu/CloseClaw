//! Plan Mode state types — shared across session and mode modules.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Plan Status — plan 生命周期状态枚举
///
/// 状态机：draft → confirmed → executing → completed
///                                  ↘ paused ↗
/// 暂停后可恢复为 executing，任何状态均可回退至 draft（拒绝/重置）。
/// 参见 `PlanState::transition_status` 合法转换表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// 草稿状态
    #[default]
    Draft,
    /// 审批通过，待执行
    Confirmed,
    /// 正在执行
    Executing,
    /// 已暂停（从 executing 或 confirmed 暂停）
    Paused,
    /// 已完成
    Completed,
}

impl std::fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Confirmed => write!(f, "confirmed"),
            Self::Executing => write!(f, "executing"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
        }
    }
}

/// 状态转换错误类型
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum StatusTransitionError {
    /// 非法状态转换
    #[error("invalid status transition: {from:?} -> {to:?}")]
    InvalidTransition { from: PlanStatus, to: PlanStatus },
}

/// Plan Path — plan 双路径选择
///
/// 标准路径（需求明确）或 Interview 路径（需求模糊）。
/// 无显式指定时由系统自动判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanPath {
    /// 标准路径：需求明确，5 阶段工作流
    Standard,
    /// Interview 路径：需求模糊，循环探索
    #[default]
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
    /// Plan 生命周期状态（权威状态源）
    #[serde(default)]
    pub status: PlanStatus,
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
    /// 显式指定的 plan 路径（None 表示由系统自动判断）
    #[serde(default)]
    pub explicit_path: Option<PlanPath>,
}

impl PlanState {
    /// 创建新的 PlanState，使用默认值（Research 阶段、空步骤、空路径）
    pub fn new() -> Self {
        Self::default()
    }

    /// 校验并执行 plan 状态转换。
    ///
    /// 合法转换：
    /// - draft → confirmed
    /// - confirmed → executing
    /// - confirmed → paused
    /// - executing → completed
    /// - executing → paused
    /// - paused → executing
    /// - 任何状态 → draft（重置/拒绝回退）
    ///
    /// 返回 `Err(StatusTransitionError::InvalidTransition)` 当转换不合法。
    pub fn transition_status(
        &mut self,
        new_status: PlanStatus,
    ) -> Result<(), StatusTransitionError> {
        if Self::is_valid_status_transition(self.status, new_status) {
            self.status = new_status;
            Ok(())
        } else {
            Err(StatusTransitionError::InvalidTransition {
                from: self.status,
                to: new_status,
            })
        }
    }

    /// 判断状态转换是否合法（不含副作用）
    fn is_valid_status_transition(from: PlanStatus, to: PlanStatus) -> bool {
        // 任何状态 → draft：允许拒绝/重置回退
        if to == PlanStatus::Draft {
            return from != PlanStatus::Draft;
        }

        matches!(
            (from, to),
            (PlanStatus::Draft, PlanStatus::Confirmed)
                | (PlanStatus::Confirmed, PlanStatus::Executing)
                | (PlanStatus::Confirmed, PlanStatus::Paused)
                | (PlanStatus::Executing, PlanStatus::Completed)
                | (PlanStatus::Executing, PlanStatus::Paused)
                | (PlanStatus::Paused, PlanStatus::Executing)
        )
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

// ---------------------------------------------------------------------------
// PlanStateWriter — plan file synchronization trait
// ---------------------------------------------------------------------------

/// Writes plan execution progress back to a plan markdown file.
///
/// Implemented by consumers who need to synchronize in-memory [`PlanState`]
/// changes to the on-disk plan file (e.g., updating status markers).
pub trait PlanStateWriter: Send + Sync {
    /// Write the current progress markers from `plan_state` into the plan
    /// markdown file at `plan_file_path`.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or written.
    fn write_progress_to_plan_file(
        &self,
        plan_file_path: &str,
        plan_state: &PlanState,
    ) -> Result<(), Box<dyn std::error::Error>>;
}

/// Default implementation of [`PlanStateWriter`] that reads a plan markdown
/// file, locates the "## 进度" progress table, and updates status markers
/// (`✅` / `🔄` / `❌` / empty) in the first column of each step row.
pub struct DefaultPlanStateWriter;

impl DefaultPlanStateWriter {
    /// Create a new `DefaultPlanStateWriter`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultPlanStateWriter {
    fn default() -> Self {
        Self
    }
}

impl PlanStateWriter for DefaultPlanStateWriter {
    fn write_progress_to_plan_file(
        &self,
        plan_file_path: &str,
        plan_state: &PlanState,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::fs;
        use std::path::Path;

        let path = Path::new(plan_file_path);
        if !path.exists() {
            return Err(format!("plan file not found: {plan_file_path}").into());
        }

        let content = fs::read_to_string(path)?;
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::with_capacity(lines.len());
        let mut in_progress_table = false;

        for line in &lines {
            if line.trim_start().starts_with("## 进度") {
                in_progress_table = true;
            }

            if in_progress_table && line.contains('|') {
                if let Some(updated) = self.update_step_row(line, plan_state) {
                    result.push(updated);
                    continue;
                }
            }

            result.push((*line).to_string());
        }

        let new_content = result.join("\n");
        fs::write(path, new_content)?;
        Ok(())
    }
}

impl DefaultPlanStateWriter {
    /// Update a single table row with the matching step's status marker.
    fn update_step_row(&self, line: &str, plan_state: &PlanState) -> Option<String> {
        // Match table rows like: | ✅ | 1.1 | ... | or | | 1.1 | ... |
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 3 {
            return None;
        }

        // The step name is in the second data column (parts[2] after
        // leading empty split element).
        let step_name = parts[2].trim();

        // Skip header and separator rows
        if step_name == "Step" || step_name == "---" || step_name.is_empty() {
            return None;
        }

        // Find matching execution step.
        // Plan table uses 1-based step numbers (1.1, 2.1, ...),
        // while step_index is 0-based.
        let matching_step = plan_state.execution_steps.iter().find(|s| {
            let prefix = format!("{}.", s.step_index + 1);
            step_name.starts_with(&prefix)
        });

        let matching_step = matching_step?;
        let marker = step_status_to_marker(&matching_step.status);

        // Rebuild the row: replace the first data column (parts[1])
        // with the new marker.
        let mut new_parts: Vec<&str> = parts.to_vec();
        new_parts[1] = &marker;

        Some(new_parts.join("|"))
    }
}

/// Map an [`ExecutionStepStatus`] to the corresponding plan file marker.
fn step_status_to_marker(status: &ExecutionStepStatus) -> String {
    match status {
        ExecutionStepStatus::Completed => "✅".to_string(),
        ExecutionStepStatus::InProgress => "🔄".to_string(),
        ExecutionStepStatus::Failed => "❌".to_string(),
        ExecutionStepStatus::Pending | ExecutionStepStatus::Skipped => String::new(),
    }
}
