//! Workflow launch port for the slash crate.
//!
//! [`WorkflowLauncher`] is a slash-owned abstraction over the workflow
//! definition loader and engine, narrowed to the exact call surface of
//! [`crate::handlers_workflow::WorkflowSlashHandler`]: a single
//! [`WorkflowLauncher::start`] call that loads the definition by name
//! (three-level lookup) and returns every datum the handler needs — the
//! system prompt context append, the Step 0 goal message, the first
//! step name, and the type-erased run handle. All session side
//! effects (active-run check, run persistence, context injection,
//! pending goal message) stay in the handler over common
//! `SlashSessionQuery`.
//!
//! The composition root (daemon) supplies the production
//! implementation assembling `closeclaw_workflow` loader + engine +
//! message builders; tests may use the thin wrapper in the
//! `real_launcher` test helper (workflow is a dev-dependency of this
//! crate).

use std::path::Path;
use thiserror::Error;

/// Data a successful workflow launch hands back to the handler.
pub struct WorkflowLaunch {
    /// Context append to inject into `system_injection_appends`
    /// (the `--- WORKFLOW ---` block rendered from the definition).
    pub context_append: String,

    /// Step 0 goal message content pushed as a pending message.
    pub goal_message: String,

    /// Name of the first step, quoted in the confirmation reply.
    pub first_step_name: String,

    /// Type-erased run handle persisted via `set_workflow_run`
    /// (`SlashSessionQuery` implementations downcast to
    /// `closeclaw_workflow::run::WorkflowRun`).
    pub run: Box<dyn std::any::Any + Send + Sync>,
}

impl std::fmt::Debug for WorkflowLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowLaunch")
            .field("context_append", &self.context_append)
            .field("goal_message", &self.goal_message)
            .field("first_step_name", &self.first_step_name)
            .field("run", &"<type-erased>")
            .finish()
    }
}

/// Error while loading a workflow definition (slash-owned mirror of
/// the workflow-side load error).
///
/// `Display` strings must stay identical to the workflow-side
/// originals — they are surfaced verbatim in replies
/// (`工作流 "{name}" 加载失败：{e}`).
#[derive(Debug, Clone, Error)]
pub enum WorkflowLoadError {
    /// The workflow definition is invalid or missing required fields.
    #[error("invalid workflow definition: {0}")]
    InvalidDefinition(String),

    /// YAML frontmatter parsing (or reading the SKILL.md) failed.
    #[error("failed to parse workflow definition: {0}")]
    ParseError(String),

    /// No matching SKILL.md at any of the three lookup levels.
    #[error("workflow definition not found: {0}")]
    DefinitionNotFound(String),

    /// Any other workflow-side error (not producible by definition
    /// loading); mapped through with its `Display` unchanged.
    #[error("{0}")]
    Other(String),
}

/// Slash-owned port over workflow definition loading and engine start.
pub trait WorkflowLauncher: Send + Sync {
    /// Load the workflow definition by name (three-level priority
    /// lookup: agent workspace → global directory → builtins) and
    /// start a run.
    fn start(
        &self,
        name: &str,
        agent_workspace: Option<&Path>,
        global_workflows: Option<&Path>,
    ) -> Result<WorkflowLaunch, WorkflowLoadError>;
}

/// Test-support thin port implementation assembling the real
/// `closeclaw_workflow` loader, engine, and message builders (workflow
/// is a dev-dependency of this crate).
///
/// Only compiled for the slash crate's own tests; the production
/// implementation lives in the daemon composition root.
#[cfg(test)]
pub(crate) mod real_launcher {
    use std::path::Path;
    use std::sync::Arc;

    use closeclaw_workflow::context_append::build_workflow_context_append;
    use closeclaw_workflow::definition::build_goal_message;
    use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;
    use closeclaw_workflow::engine::WorkflowEngine;
    use closeclaw_workflow::error::WorkflowError;
    use closeclaw_workflow::run::GoalHint;

    use super::{WorkflowLaunch, WorkflowLauncher, WorkflowLoadError};

    /// Map a workflow-side error to the slash-side mirror, preserving
    /// the payload (and therefore the `Display` output) verbatim.
    pub(crate) fn load_error_to_slash(e: WorkflowError) -> WorkflowLoadError {
        match e {
            WorkflowError::InvalidDefinition(m) => WorkflowLoadError::InvalidDefinition(m),
            WorkflowError::ParseError(m) => WorkflowLoadError::ParseError(m),
            WorkflowError::DefinitionNotFound(n) => WorkflowLoadError::DefinitionNotFound(n),
            other => WorkflowLoadError::Other(other.to_string()),
        }
    }

    /// Thin wrapper mirroring the daemon's production adapter.
    pub(crate) struct RealWorkflowLauncher;

    impl WorkflowLauncher for RealWorkflowLauncher {
        fn start(
            &self,
            name: &str,
            agent_workspace: Option<&Path>,
            global_workflows: Option<&Path>,
        ) -> Result<WorkflowLaunch, WorkflowLoadError> {
            let workflow = WorkflowDefinitionLoader::load(name, agent_workspace, global_workflows)
                .map_err(load_error_to_slash)?;
            let run = WorkflowEngine::start(&workflow);
            let goal_message = build_goal_message(&workflow.steps[0], GoalHint::Normal);
            let context_append = build_workflow_context_append(&workflow);
            let first_step_name = workflow.steps[0].name.clone();
            Ok(WorkflowLaunch {
                context_append,
                goal_message,
                first_step_name,
                run: Box::new(run),
            })
        }
    }

    /// Build the real launcher as `Arc<dyn WorkflowLauncher>` for tests.
    pub(crate) fn real_launcher() -> Arc<dyn WorkflowLauncher> {
        Arc::new(RealWorkflowLauncher)
    }
}
