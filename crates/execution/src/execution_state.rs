//! Execution state — runtime state for the plan execution engine.
//!
//! Contains the execution step state machine fields that were previously
//! part of `PlanState`. These are consumed by the execution engine,
//! tools (ProgressTool), and session (recovery).

use serde::{Deserialize, Serialize};

use closeclaw_common::{ExecutionStep, ExecutionStepStatus, PlanPath, TransitionError};

/// Execution state — runtime state for plan step execution.
///
/// Holds the execution step list, current step pointer, step selection,
/// and explicit plan path. This state is separate from [`PlanState`]
/// (which only carries plan-level metadata) and is managed by the
/// execution engine and ProgressTool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionState {
    /// 执行步骤列表
    #[serde(default)]
    pub execution_steps: Vec<ExecutionStep>,
    /// 当前正在执行的步骤索引
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

impl ExecutionState {
    /// Create a new empty ExecutionState.
    pub fn new() -> Self {
        Self::default()
    }

}

/// 根据步骤描述列表初始化执行步骤（全部 pending），
/// 重置 current_step = None
pub fn init_execution_steps(state: &mut ExecutionState, steps: Vec<String>) {
    state.execution_steps = steps
        .into_iter()
        .enumerate()
        .map(|(i, s)| ExecutionStep {
            step_index: i,
            status: ExecutionStepStatus::Pending,
            summary: s,
            error_message: None,
        })
        .collect();
    state.current_step = None;
}

/// 获取指定步骤的状态
pub fn get_step_status(state: &ExecutionState, step_index: usize) -> Option<&ExecutionStepStatus> {
    state.execution_steps.get(step_index).map(|s| &s.status)
}

/// 获取当前步骤索引
pub fn current_step_index(state: &ExecutionState) -> Option<usize> {
    state.current_step
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
pub fn progress_summary(state: &ExecutionState) -> String {
    if state.execution_steps.is_empty() {
        return String::new();
    }
    let total = state.execution_steps.len();
    let mut lines = Vec::with_capacity(total + 1);
    lines.push("## Execution Progress".to_string());
    for step in &state.execution_steps {
        let idx = step.step_index + 1;
        let is_current = state.current_step == Some(step.step_index);
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
    state: &ExecutionState,
    step_index: usize,
    new_status: &ExecutionStepStatus,
) -> Result<(), TransitionError> {
    let steps_len = state.execution_steps.len();
    if step_index >= steps_len {
        return Err(TransitionError::OutOfBounds {
            index: step_index,
            len: steps_len,
        });
    }

    let current = &state.execution_steps[step_index].status;

    // Skipped → InProgress: skip the step-order check so that a
    // previously-skipped step can be resumed even when current_step
    // has already advanced past it.
    if *current == ExecutionStepStatus::Skipped && new_status == &ExecutionStepStatus::InProgress {
        return Ok(());
    }

    // Skip-step check: step_index must == current_step (if set) or == 0
    if let Some(cur) = state.current_step {
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
    let valid = match new_status {
        ExecutionStepStatus::InProgress => {
            matches!(
                current,
                ExecutionStepStatus::Pending
                    | ExecutionStepStatus::Failed
                    | ExecutionStepStatus::Skipped
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
    state: &mut ExecutionState,
    step_index: usize,
    new_status: ExecutionStepStatus,
) -> Result<(), TransitionError> {
    validate_transition(state, step_index, &new_status)?;
    let old_status = state.execution_steps[step_index].status;
    state.execution_steps[step_index].status = new_status;

    // Update current_step based on new status
    if matches!(
        new_status,
        ExecutionStepStatus::Completed | ExecutionStepStatus::Skipped
    ) {
        let next = step_index + 1;
        if next < state.execution_steps.len() {
            state.current_step = Some(next);
        }
    } else if new_status == ExecutionStepStatus::InProgress
        && old_status == ExecutionStepStatus::Skipped
    {
        // When resuming from Skipped, point current_step back to this step
        state.current_step = Some(step_index);
    }
    // Failed / Pending→InProgress: keep current_step unchanged

    Ok(())
}

// ---------------------------------------------------------------------------
// PlanStateWriter — plan file synchronization trait
// ---------------------------------------------------------------------------

/// Writes plan execution progress back to a plan markdown file.
///
/// Implemented by consumers who need to synchronize in-memory [`ExecutionState`]
/// changes to the on-disk plan file (e.g., updating status markers).
pub trait PlanStateWriter: Send + Sync {
    /// Write the current progress markers from `execution_state` into the plan
    /// markdown file at `plan_file_path`.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or written.
    fn write_progress_to_plan_file(
        &self,
        plan_file_path: &str,
        execution_state: &ExecutionState,
    ) -> Result<(), Box<dyn std::error::Error>>;
}

/// Default implementation of [`PlanStateWriter`] that reads a plan markdown
/// file, locates the "## Tasks" section, and updates status markers
/// (`[x]` / `[-]` / `[!]` / `[ ]`) in the first column of each step row.
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
        execution_state: &ExecutionState,
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
        let mut in_tasks_section = false;

        for line in &lines {
            if line.trim_start().starts_with("## ") {
                in_tasks_section = line.trim_start().starts_with("## Tasks");
            }

            if in_tasks_section && line.contains('|') {
                if let Some(updated) = self.update_step_row(line, execution_state) {
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
    fn update_step_row(&self, line: &str, execution_state: &ExecutionState) -> Option<String> {
        // Match table rows like: | [-] | 1.1 | ... | or | [ ] | 1.1 | ... |
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
        let matching_step = execution_state.execution_steps.iter().find(|s| {
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
///
/// Uses GitHub-flavored Markdown checkbox syntax per design doc:
/// - `Completed` → `[x]`
/// - `InProgress` → `[-]`
/// - `Failed` → `[!]`
/// - `Pending` → `[ ]`
/// - `Skipped` → `[~]`
pub fn step_status_to_marker(status: &ExecutionStepStatus) -> String {
    match status {
        ExecutionStepStatus::Completed => "[x]".to_string(),
        ExecutionStepStatus::InProgress => "[-]".to_string(),
        ExecutionStepStatus::Failed => "[!]".to_string(),
        ExecutionStepStatus::Pending => "[ ]".to_string(),
        ExecutionStepStatus::Skipped => "[~]".to_string(),
    }
}
