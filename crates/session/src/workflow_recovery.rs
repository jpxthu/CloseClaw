//! Workflow recovery state injection during session recovery.
//!
//! Detects active workflow runs in recovered checkpoints and injects
//! workflow context and recovery notifications into `system_appends`.

use crate::persistence::SessionCheckpoint;
use closeclaw_workflow::context_append::{build_workflow_context_append, has_workflow_context};
use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;
use closeclaw_workflow::run::Phase;

/// Prefix marker for workflow recovery notification in `system_appends`.
pub const WORKFLOW_RECOVERY_PREFIX: &str = "__workflow_recovery__:";

/// Inject workflow recovery state for sessions with active workflow runs.
///
/// When a checkpoint contains a `workflow_run` with phase != Complete:
/// 1. Re-injects workflow context into `system_appends` (if not already present)
/// 2. Stores a recovery notification with step information
/// 3. Handles definition_version changes (transitions to blocked if current
///    step no longer exists in the new definition)
pub async fn inject_workflow_recovery(session_id: &str, checkpoint: &mut SessionCheckpoint) {
    let wf_run = match &checkpoint.workflow_run {
        Some(run) if run.phase != Phase::Complete => run.clone(),
        _ => return,
    };

    let wf = try_reload_definition(&wf_run.definition_name);

    // 1. Re-inject workflow context into system_appends if not already present
    if !has_workflow_context(&checkpoint.system_appends) {
        if let Some(ref wf) = wf {
            checkpoint
                .system_appends
                .push(build_workflow_context_append(wf));
        } else {
            tracing::warn!(
                session_id = %session_id,
                definition_name = %wf_run.definition_name,
                "failed to reload workflow definition for context re-injection"
            );
        }
    }

    let step_num = wf_run.current_step;
    let step_name = wf_run
        .step_history
        .last()
        .map(|e| e.step_name.as_str())
        .unwrap_or("unknown");
    let notification = build_recovery_notification(&wf_run.definition_name, step_num, step_name);

    // 2. Handle definition_version changes
    handle_definition_version_change(session_id, &wf, &wf_run, checkpoint);

    // 3. Store recovery notification in system_appends
    let tagged = format!("{}{}", WORKFLOW_RECOVERY_PREFIX, notification);
    if let Some(slot) = checkpoint
        .system_appends
        .iter_mut()
        .find(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX))
    {
        *slot = tagged;
    } else {
        checkpoint.system_appends.push(tagged);
    }

    tracing::info!(
        session_id = %session_id,
        workflow_name = %wf_run.definition_name,
        step = step_num,
        phase = ?wf_run.phase,
        "injected workflow recovery state into system_appends"
    );
}

/// Try to reload the workflow definition from disk.
fn try_reload_definition(
    definition_name: &str,
) -> Option<closeclaw_workflow::definition::Workflow> {
    WorkflowDefinitionLoader::load(definition_name, None, None).ok()
}

/// Build a recovery notification string summarising the current workflow state.
fn build_recovery_notification(definition_name: &str, step_num: usize, step_name: &str) -> String {
    format!(
        "[workflow recovered] 正在执行 {name}，当前 Step {step} ({step_name})",
        name = definition_name,
        step = step_num,
        step_name = step_name,
    )
}

/// Handle definition_version changes — block the workflow if the current
/// step no longer exists in the new definition.
fn handle_definition_version_change(
    session_id: &str,
    wf: &Option<closeclaw_workflow::definition::Workflow>,
    wf_run: &closeclaw_workflow::run::WorkflowRun,
    checkpoint: &mut SessionCheckpoint,
) {
    let Some(ref wf) = wf else {
        return;
    };
    if wf.version.as_deref() == Some(&wf_run.definition_version) {
        return;
    }
    tracing::info!(
        session_id = %session_id,
        old_version = %wf_run.definition_version,
        new_version = ?wf.version,
        "workflow definition version changed during recovery"
    );
    let step_num = wf_run.current_step;
    if step_num >= wf.steps.len() {
        tracing::warn!(
            session_id = %session_id,
            step_num,
            total_steps = wf.steps.len(),
            "current step not in new definition — blocking workflow"
        );
        checkpoint
            .workflow_run
            .as_mut()
            .expect("workflow_run checked above")
            .phase = Phase::Blocked;
    }
}

/// Clean up all workflow-related state from a session checkpoint.

/// Performs the four cleanup steps required by the workflow exit flow:
///
/// 1. Remove workflow context markers from `system_appends`
///    (items starting with `"--- WORKFLOW ---"`).
/// 2. Remove workflow recovery notification entries from `system_appends`
///    (items starting with [`WORKFLOW_RECOVERY_PREFIX`]).
/// 3. Set `workflow_run` to `None`.
/// 4. The caller is responsible for persisting the checkpoint after
///    calling this method.
///
/// This method does **not** handle message-history cleanup — that is
/// the responsibility of the session layer (`ConversationSession`),
/// which owns the in-memory transcript.
///
/// # Returns
///
/// A [`WorkflowExitReport`] summarising what was cleaned up.
pub fn cleanup_workflow_exit(checkpoint: &mut SessionCheckpoint) -> WorkflowExitReport {
    let mut report = WorkflowExitReport::default();

    // 1. Remove workflow context markers from system_appends.
    report.removed_contexts =
        closeclaw_workflow::context_append::remove_workflow_context(&mut checkpoint.system_appends);

    // 2. Remove workflow recovery notification entries.
    let before = checkpoint.system_appends.len();
    checkpoint
        .system_appends
        .retain(|s| !s.starts_with(WORKFLOW_RECOVERY_PREFIX));
    report.removed_recovery_notifications = before - checkpoint.system_appends.len();

    // 3. Clear workflow_run.
    if checkpoint.workflow_run.is_some() {
        report.had_workflow_run = true;
        checkpoint.workflow_run = None;
    }

    tracing::debug!(
        removed_contexts = report.removed_contexts,
        removed_recovery_notifications = report.removed_recovery_notifications,
        had_workflow_run = report.had_workflow_run,
        "workflow exit cleanup applied to checkpoint"
    );

    report
}

/// Summary of what [`cleanup_workflow_exit`] cleaned up from a checkpoint.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkflowExitReport {
    /// Number of workflow context markers removed from `system_appends`.
    pub removed_contexts: usize,
    /// Number of recovery notification entries removed from `system_appends`.
    pub removed_recovery_notifications: usize,
    /// Whether a `workflow_run` was present (and now cleared).
    pub had_workflow_run: bool,
}
