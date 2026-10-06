//! Workflow recovery state injection during session recovery.
//!
//! Detects active workflow runs in recovered checkpoints and injects
//! workflow context and recovery notifications into `system_injection_appends`.
//!
//! All workflow-engine operations (definition loading, message
//! rendering, state mutation) are delegated to the injected
//! [`WorkflowPort`]; this module keeps the recovery orchestration,
//! notification formatting, and cleanup ordering.

use crate::persistence::SessionCheckpoint;
use crate::workflow_port::{WorkflowGoalHint, WorkflowPhase, WorkflowPort, WorkflowRunInfo};

/// Prefix marker for workflow recovery notification in `system_injection_appends`.
pub const WORKFLOW_RECOVERY_PREFIX: &str = "__workflow_recovery__:";

/// Fixed paused_reason text set when a workflow step no longer exists in
/// the latest definition version. Used as a single source of truth for
/// both the write site (recovery) and the read site (owner resolve guard).
pub const DEFINITION_CHANGED_PAUSE_REASON: &str = "当前步骤在最新定义中已不存在";

/// Prefix marker for workflow context items in `system_injection_appends`.
const WORKFLOW_CONTEXT_PREFIX: &str = "--- WORKFLOW ---";

/// Returns `true` when a workflow context marker exists in the list.
fn has_workflow_context_marker(appends: &[String]) -> bool {
    appends
        .iter()
        .any(|s| s.starts_with(WORKFLOW_CONTEXT_PREFIX))
}

/// Remove all workflow context markers from the list, returning the
/// number of removed items.
fn remove_workflow_context_markers(appends: &mut Vec<String>) -> usize {
    let before = appends.len();
    appends.retain(|s| !s.starts_with(WORKFLOW_CONTEXT_PREFIX));
    before - appends.len()
}

/// Inject workflow recovery state for sessions with active workflow runs.
///
/// When a checkpoint contains a `workflow_run` with phase != Complete:
/// 1. Re-injects workflow context into `system_injection_appends` (if not already present)
/// 2. Stores a recovery notification with step information
/// 3. Handles definition_version changes (transitions to blocked if current
///    step no longer exists in the new definition)
pub async fn inject_workflow_recovery(
    session_id: &str,
    checkpoint: &mut SessionCheckpoint,
    agent_workspace: Option<&std::path::Path>,
    port: &dyn WorkflowPort,
) {
    let Some(mut state) = checkpoint.workflow_run.clone() else {
        return;
    };
    let Some(mut info) = port.run_info(&state) else {
        tracing::warn!(
            error = "undecodable",
            "failed to decode checkpoint workflow_run"
        );
        return;
    };
    if info.phase == WorkflowPhase::Complete {
        return;
    }

    let wf = try_reload_definition(port, &info.definition_name, agent_workspace);

    // 1. Re-inject workflow context into system_injection_appends if not already present
    if !has_workflow_context_marker(&checkpoint.system_injection_appends) {
        if let Some(ref wf) = wf {
            checkpoint
                .system_injection_appends
                .push(port.build_context_append(wf));
        } else {
            tracing::warn!(
                session_id = %session_id,
                definition_name = %info.definition_name,
                "failed to reload workflow definition for context re-injection"
            );
        }
    }

    // 2. Handle definition_version changes
    handle_definition_version_change(session_id, port, &wf, &mut state, &mut info);
    checkpoint.workflow_run = Some(state);

    // 3. Extract step info and store recovery notification
    //    Read from the updated info above (handle_definition_version_change
    //    may have updated phase/paused_reason on it).
    let (step_num, definition_name, step_name, phase, paused_reason) = (
        info.current_step,
        info.definition_name.clone(),
        info.last_history_step_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        info.phase,
        info.paused_reason.clone(),
    );
    // Build recovery workflow messages (recovered + goal) for transcript injection.
    // store_recovery_notification remains for system prompt context.
    build_recovery_workflow_messages(
        port,
        &wf,
        &info,
        checkpoint,
        &step_name,
        phase,
        &paused_reason,
    );
    store_recovery_notification(
        &definition_name,
        step_num,
        &step_name,
        phase,
        &paused_reason,
        checkpoint,
    );

    tracing::info!(
        session_id = %session_id,
        workflow_name = %definition_name,
        step = step_num,
        phase = ?phase,
        "injected workflow recovery state into system_injection_appends"
    );
}

/// Try to reload the workflow definition from disk via the port.
fn try_reload_definition(
    port: &dyn WorkflowPort,
    definition_name: &str,
    agent_workspace: Option<&std::path::Path>,
) -> Option<serde_json::Value> {
    let global_workflows = dirs::home_dir().map(|h| h.join(".openclaw"));
    port.load_definition(
        definition_name,
        agent_workspace,
        global_workflows.as_deref(),
    )
    .ok()
}

/// Store a recovery notification in `system_injection_appends`.
fn store_recovery_notification(
    definition_name: &str,
    step_num: usize,
    step_name: &str,
    phase: WorkflowPhase,
    paused_reason: &str,
    checkpoint: &mut SessionCheckpoint,
) {
    let reason = if phase == WorkflowPhase::Blocked && !paused_reason.is_empty() {
        Some(paused_reason)
    } else {
        None
    };
    let notification = build_recovery_notification(definition_name, step_num, step_name, reason);
    let tagged = format!("{}{}", WORKFLOW_RECOVERY_PREFIX, notification);
    if let Some(slot) = checkpoint
        .system_injection_appends
        .iter_mut()
        .find(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX))
    {
        *slot = tagged;
    } else {
        checkpoint.system_injection_appends.push(tagged);
    }
}

/// Build a recovery notification string summarising the current workflow state.
///
/// When `paused_reason` is `Some`, appends the pause reason (for blocked-phase
/// recovery); otherwise returns a plain recovery notification.
fn build_recovery_notification(
    definition_name: &str,
    step_num: usize,
    step_name: &str,
    paused_reason: Option<&str>,
) -> String {
    let base = format!(
        "[workflow recovered] 正在执行 {name}，当前 Step {step} ({step_name})",
        name = definition_name,
        step = step_num,
        step_name = step_name,
    );
    match paused_reason {
        Some(reason) => format!("{}\n暂停原因：{}", base, reason),
        None => base,
    }
}

/// Handle definition_version changes — block the workflow if the current
/// step no longer exists in the new definition.
fn handle_definition_version_change(
    session_id: &str,
    port: &dyn WorkflowPort,
    wf: &Option<serde_json::Value>,
    state: &mut serde_json::Value,
    info: &mut WorkflowRunInfo,
) {
    let Some(ref wf) = wf else {
        return;
    };
    let definition_version = port.definition_version(wf);
    if definition_version.as_deref() == Some(info.definition_version.as_str()) {
        return;
    }
    tracing::info!(
        session_id = %session_id,
        old_version = %info.definition_version,
        new_version = ?definition_version,
        "workflow definition version changed during recovery"
    );
    let step_num = info.current_step;
    if step_num >= port.definition_step_count(wf) {
        tracing::warn!(
            session_id = %session_id,
            step_num,
            total_steps = port.definition_step_count(wf),
            "current step not in new definition — blocking workflow"
        );
        *state = port.mark_blocked(std::mem::take(state), DEFINITION_CHANGED_PAUSE_REASON);
        info.phase = WorkflowPhase::Blocked;
        info.paused_reason = DEFINITION_CHANGED_PAUSE_REASON.to_string();
    }
}

/// Build recovery workflow messages (recovered + goal) and store them
/// in `checkpoint.recovery_workflow_messages` for Gateway transcript injection.
///
/// When the definition could not be loaded from disk (`wf` is `None`),
/// only the recovered message is built (goal requires step definitions).
/// Goal message is only built when the step exists in the latest definition.
fn build_recovery_workflow_messages(
    port: &dyn WorkflowPort,
    wf: &Option<serde_json::Value>,
    info: &WorkflowRunInfo,
    checkpoint: &mut SessionCheckpoint,
    step_name: &str,
    phase: WorkflowPhase,
    paused_reason: &str,
) {
    let step_num = info.current_step;

    // Recovered message (always built)
    let mut recovered_msg = format!(
        "[workflow recovered] 正在执行 {}，当前 Step {} ({})",
        info.definition_name, step_num, step_name
    );
    if phase == WorkflowPhase::Blocked && !paused_reason.is_empty() {
        recovered_msg = format!("{}\n暂停原因：{}", recovered_msg, paused_reason);
    }

    // Goal message (only when definition loaded and step exists)
    let goal_msg = wf
        .as_ref()
        .and_then(|wf_def| port.goal_message(wf_def, step_num, WorkflowGoalHint::Normal));

    let mut msgs = vec![recovered_msg];
    if let Some(ref goal) = goal_msg {
        msgs.push(goal.clone());
    }

    // Re-inject jump question when phase is Jumping (mirrors initial injection).
    if phase == WorkflowPhase::Jumping {
        if let Some(def) = wf.as_ref() {
            let jump_msg = checkpoint
                .workflow_run
                .clone()
                .and_then(|state| port.recovery_jump_message(&state, def));
            if let Some(jump_msg) = jump_msg {
                msgs.push(jump_msg);
            }
        }
    }

    tracing::debug!(
        step = step_num,
        goal_built = goal_msg.is_some(),
        "built recovery workflow messages"
    );

    checkpoint.recovery_workflow_messages = msgs;
}

/// Clean up all workflow-related state from a session checkpoint.
///
/// Performs the four cleanup steps required by the workflow exit flow:
///
/// 1. Remove workflow context markers from `system_injection_appends`
///    (items starting with `"--- WORKFLOW ---"`).
/// 2. Remove workflow recovery notification entries from `system_injection_appends`
///    (items starting with [`WORKFLOW_RECOVERY_PREFIX`]).
/// 3. Set `workflow_run` to `None`.
/// 4. Clear `recovery_workflow_messages`.
///
/// This method does **not** handle message-history cleanup — that is
/// the responsibility of the session layer (`ConversationSession`),
/// which owns the in-memory transcript.
///
/// # Returns
///
/// A [`WorkflowExitReport`] summarising what was cleaned up.
pub fn cleanup_workflow_exit(checkpoint: &mut SessionCheckpoint) -> WorkflowExitReport {
    // 1. Remove workflow context markers from system_injection_appends.
    let removed_contexts =
        remove_workflow_context_markers(&mut checkpoint.system_injection_appends);

    // 2. Remove workflow recovery notification entries.
    let before = checkpoint.system_injection_appends.len();
    checkpoint
        .system_injection_appends
        .retain(|s| !s.starts_with(WORKFLOW_RECOVERY_PREFIX));
    let removed_recovery_notifications = before - checkpoint.system_injection_appends.len();

    // 3. Clear workflow_run.
    let had_workflow_run = checkpoint.workflow_run.is_some();
    if had_workflow_run {
        checkpoint.workflow_run = None;
    }

    // 4. Clear recovery_workflow_messages.
    checkpoint.recovery_workflow_messages.clear();

    tracing::debug!(
        removed_contexts,
        removed_recovery_notifications,
        had_workflow_run,
        "workflow exit cleanup applied to checkpoint"
    );

    WorkflowExitReport {
        removed_contexts,
        removed_recovery_notifications,
        had_workflow_run,
    }
}

/// Summary of what [`cleanup_workflow_exit`] cleaned up from a checkpoint.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkflowExitReport {
    /// Number of workflow context markers removed from `system_injection_appends`.
    pub removed_contexts: usize,
    /// Number of recovery notification entries removed from `system_injection_appends`.
    pub removed_recovery_notifications: usize,
    /// Whether a `workflow_run` was present (and now cleared).
    pub had_workflow_run: bool,
}
