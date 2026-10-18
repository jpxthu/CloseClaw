//! WorkflowControl implementation composing the pure engine with session I/O.
//!
//! [`WorkflowEngineControl`] implements the common
//! [`WorkflowControl`](closeclaw_common::WorkflowControl) trait: it wires the
//! pure [`WorkflowEngine`] state machine to session persistence and message
//! injection through two injected ports:
//!
//! - [`SlashSessionQuery`](closeclaw_common::SlashSessionQuery) (implemented by
//!   the gateway SessionManager): active-run check, run persistence, workdir
//!   lookup, system injection appends, pending message injection.
//! - [`WorkflowSessionOps`] (workflow-owned port, implemented by the daemon
//!   adapter): type-erased run reads, pause-kind classification, workflow
//!   control message / context append cleanup. The port keeps this crate from
//!   depending on the session crate for `DEFINITION_CHANGED_PAUSE_REASON`.
//!
//! Signal semantics follow `docs/design/common/core-traits.md#workflowcontrol`
//! and `docs/design/workflow/execution-engine.md` (阻塞处理 / Owner 终止).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use closeclaw_common::session_lookup::PendingMessage;
use closeclaw_common::{SlashSessionQuery, WorkflowControl};

use crate::context_append::build_workflow_context_append;
use crate::definition::{build_goal_message, build_verify_message, Step, Workflow};
use crate::definition_loader::WorkflowDefinitionLoader;
use crate::engine::WorkflowEngine;
use crate::run::{GoalHint, WorkflowRun};

/// Classification of a blocked workflow run from the owner-resolution
/// perspective, mirroring execution-engine.md「阻塞处理」.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerPauseKind {
    /// No blocked run (or no run at all) — owner messages are a no-op.
    NotBlocked,
    /// Resumable pause (agent-initiated block or verify-retry exhaustion) —
    /// an owner non-slash message lifts the block.
    Resolvable,
    /// Terminable pause (definition version change) — only owner `/stop`
    /// may end it; owner messages are a no-op.
    Terminable,
}

/// Session-side workflow operations port, implemented by the daemon adapter.
///
/// Keeps the control layer free of session-crate dependencies: implementors
/// bridge to the live `ConversationSession` (read the type-erased run, classify
/// the pause kind against the definition-change reason, clean control messages
/// and the workflow context append).
#[async_trait]
pub trait WorkflowSessionOps: Send + Sync {
    /// Classify the session's workflow pause (see [`OwnerPauseKind`]).
    async fn owner_pause_kind(&self, session_id: &str) -> OwnerPauseKind;

    /// Read the type-erased (serialized checkpoint) workflow run, if any.
    async fn read_workflow_run(&self, session_id: &str) -> Option<serde_json::Value>;

    /// Remove residual verify checklist messages from the transcript,
    /// preserving goal and recovered messages.
    async fn remove_workflow_verify_messages(&self, session_id: &str);

    /// Remove all workflow control messages from the transcript.
    async fn remove_workflow_messages(&self, session_id: &str);

    /// Remove the workflow context (`--- WORKFLOW ---` items) from the
    /// system injection appends.
    async fn remove_workflow_context_append(&self, session_id: &str);
}

/// Engine-side [`WorkflowControl`] implementation.
///
/// Stateless orchestration over the injected ports; workflow definitions are
/// resolved per call via the three-level [`WorkflowDefinitionLoader`] lookup
/// (session workdir → `global_workflows` → built-ins).
pub struct WorkflowEngineControl {
    sessions: Arc<dyn SlashSessionQuery>,
    ops: Arc<dyn WorkflowSessionOps>,
    global_workflows: Option<PathBuf>,
}

impl WorkflowEngineControl {
    /// Create a control wiring the engine to the given session query and
    /// session-ops ports. `global_workflows` is the level-2 definition
    /// lookup root (e.g. `$HOME/.openclaw`).
    pub fn new(
        sessions: Arc<dyn SlashSessionQuery>,
        ops: Arc<dyn WorkflowSessionOps>,
        global_workflows: Option<PathBuf>,
    ) -> Self {
        Self {
            sessions,
            ops,
            global_workflows,
        }
    }

    /// Load the latest definition for `name` (session workdir → global
    /// directory → built-ins).
    async fn load_definition(
        &self,
        session_id: &str,
        name: &str,
    ) -> Result<Workflow, crate::error::WorkflowError> {
        let workdir = self.sessions.get_workdir(session_id).await;
        WorkflowDefinitionLoader::load(name, workdir.as_deref(), self.global_workflows.as_deref())
    }

    /// Push a workflow-role pending message into the unified message queue.
    async fn push_workflow_message(
        &self,
        session_id: &str,
        message_id: String,
        content: String,
    ) -> Result<(), String> {
        let msg = PendingMessage::with_role(message_id, content, "workflow".to_string());
        self.sessions.push_pending_message(session_id, msg).await
    }
}

/// Persistence and message-rendering helpers over the run state.
impl WorkflowEngineControl {
    /// Serialize and persist a run snapshot.
    async fn persist_run(&self, session_id: &str, run: &WorkflowRun) -> Result<(), String> {
        let value =
            serde_json::to_value(run).map_err(|e| format!("工作流运行状态序列化失败：{e}"))?;
        self.sessions
            .set_workflow_run(session_id, Some(Box::new(value)))
            .await
            .map_err(|e| format!("工作流运行状态持久化失败：{e}"))
    }

    /// Decode the stored type-erased run into the typed form.
    fn decode_run(value: serde_json::Value) -> Result<WorkflowRun, String> {
        serde_json::from_value(value).map_err(|e| format!("工作流运行状态解码失败：{e}"))
    }

    /// Look up the run's current step in the (latest) definition.
    fn current_step<'a>(run: &WorkflowRun, workflow: &'a Workflow) -> Result<&'a Step, String> {
        workflow.steps.get(run.current_step).ok_or_else(|| {
            format!(
                "工作流当前步骤 {} 在最新定义中不存在：{}",
                run.current_step, workflow.name
            )
        })
    }

    /// Refresh the step goal message when the definition changed.
    ///
    /// Unchanged definition: only clear residual verify messages (the goal
    /// message is preserved). Changed definition: clear all workflow control
    /// messages and re-inject the goal per the latest definition, updating the
    /// run's `definition_version` to the latest.
    async fn refresh_goal_if_changed(
        &self,
        session_id: &str,
        run: &mut WorkflowRun,
        workflow: &Workflow,
        step: &Step,
    ) -> Result<(), String> {
        let latest_version = workflow
            .version
            .clone()
            .unwrap_or_else(|| "0.1".to_string());
        if run.definition_version == latest_version {
            self.ops.remove_workflow_verify_messages(session_id).await;
            return Ok(());
        }
        self.ops.remove_workflow_messages(session_id).await;
        let goal = build_goal_message(step, run.pending_goal_hint.clone());
        let message_id = format!("workflow-goal-{session_id}");
        self.push_workflow_message(session_id, message_id, goal)
            .await?;
        WorkflowEngine::on_goal_injected(run);
        run.definition_version = latest_version;
        Ok(())
    }

    /// Inject the verify checklist for the run's current step.
    async fn inject_verify_checklist(
        &self,
        session_id: &str,
        workflow: &Workflow,
        step: &Step,
    ) -> Result<(), String> {
        let allow_blocked = step.allow_blocked.unwrap_or(workflow.allow_blocked);
        let checklist = build_verify_message(step, allow_blocked);
        let message_id = format!("workflow-verify-{session_id}");
        self.push_workflow_message(session_id, message_id, checklist)
            .await
    }
}

#[async_trait]
impl WorkflowControl for WorkflowEngineControl {
    async fn start(&self, session_id: &str, workflow_name: &str) -> Result<(), String> {
        if let Some(phase) = self
            .sessions
            .get_active_workflow_run_phase(session_id)
            .await
        {
            return Err(format!(
                "无法启动工作流：当前 Session 已有活跃的工作流运行（阶段：{phase}）。\
                 请等待当前工作流完成或由 Owner 终止后再启动新工作流。"
            ));
        }
        let workflow = self
            .load_definition(session_id, workflow_name)
            .await
            .map_err(|e| format!("工作流 \"{workflow_name}\" 加载失败：{e}"))?;
        let run = WorkflowEngine::start(&workflow);
        self.persist_run(session_id, &run)
            .await
            .map_err(|e| format!("工作流 \"{workflow_name}\" 启动失败（持久化错误）：{e}"))?;
        self.sessions
            .add_system_injection_append(session_id, build_workflow_context_append(&workflow))
            .await;
        let goal = build_goal_message(&workflow.steps[0], GoalHint::Normal);
        let message_id = format!("workflow-goal-{session_id}");
        self.push_workflow_message(session_id, message_id, goal)
            .await
            .map_err(|e| format!("工作流 \"{workflow_name}\" 启动失败（消息注入错误）：{e}"))?;
        Ok(())
    }

    async fn on_owner_message(&self, session_id: &str) -> Result<(), String> {
        if self.ops.owner_pause_kind(session_id).await != OwnerPauseKind::Resolvable {
            return Ok(());
        }
        let Some(run_value) = self.ops.read_workflow_run(session_id).await else {
            return Ok(());
        };
        let mut run = Self::decode_run(run_value)?;
        let workflow = self
            .load_definition(session_id, &run.definition_name)
            .await
            .map_err(|e| format!("工作流 \"{}\" 定义加载失败：{e}", run.definition_name))?;
        let step = Self::current_step(&run, &workflow)?;
        self.refresh_goal_if_changed(session_id, &mut run, &workflow, step)
            .await?;
        WorkflowEngine::on_owner_resolve(&mut run);
        self.inject_verify_checklist(session_id, &workflow, step)
            .await?;
        self.persist_run(session_id, &run).await
    }

    async fn terminate(&self, session_id: &str) -> Result<(), String> {
        if self
            .sessions
            .get_active_workflow_run_phase(session_id)
            .await
            .is_none()
        {
            return Ok(());
        }
        self.ops.remove_workflow_context_append(session_id).await;
        self.ops.remove_workflow_messages(session_id).await;
        self.sessions
            .set_workflow_run(session_id, None)
            .await
            .map_err(|e| format!("工作流运行状态清理失败：{e}"))
    }
}
