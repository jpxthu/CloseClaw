//! Slash handler for `/workflow <name>`.
//!
//! Loads a workflow definition, initializes a WorkflowRun, persists it
//! to the session checkpoint, injects the workflow context into the
//! system prompt append section, and pushes the Step 0 goal message.

use std::path::PathBuf;
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::session_lookup::PendingMessage;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::SlashSessionQuery;
use closeclaw_workflow::context_append::build_workflow_context_append;
use closeclaw_workflow::definition::Workflow;
use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;
use closeclaw_workflow::engine::WorkflowEngine;

/// `/workflow <name>` — start a workflow by definition name.
///
/// Processing flow:
/// 1. Extract `name` parameter
/// 2. Load workflow definition via three-level lookup
/// 3. Initialize WorkflowRun via WorkflowEngine::start
/// 4. Persist WorkflowRun to session checkpoint
/// 5. Inject workflow context into system_appends
/// 6. Push Step 0 goal message as pending
/// 7. Return confirmation
#[derive(Clone)]
pub struct WorkflowSlashHandler {
    session_manager: Arc<dyn SlashSessionQuery>,
    agent_workspace: Option<PathBuf>,
    dot_closeclaw: Option<PathBuf>,
}

impl WorkflowSlashHandler {
    /// Create a new WorkflowHandler.
    pub fn new(
        session_manager: Arc<dyn SlashSessionQuery>,
        agent_workspace: Option<PathBuf>,
        dot_closeclaw: Option<PathBuf>,
    ) -> Self {
        Self {
            session_manager,
            agent_workspace,
            dot_closeclaw,
        }
    }

    /// Build the workflow context string to inject into system_appends.
    ///
    /// Delegates to [`closeclaw_workflow::context_append::build_workflow_context_append`].
    pub fn build_workflow_context_append(workflow: &Workflow) -> String {
        build_workflow_context_append(workflow)
    }

    /// Build the Step 0 goal message content.
    fn build_goal_message(workflow: &Workflow) -> String {
        let step = &workflow.steps[0];
        format!(
            "[workflow goal] Step {id}: {name}\n\n{goal}",
            id = step.id,
            name = step.name,
            goal = step.goal,
        )
    }
}

#[async_trait::async_trait]
impl SlashHandler for WorkflowSlashHandler {
    fn commands(&self) -> &[&str] {
        &["workflow"]
    }

    fn description(&self) -> &str {
        "启动受控工作流"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let name = args.trim();
        if name.is_empty() {
            return SlashResult::Reply("用法：/workflow <name>".to_owned());
        }
        let workflow = match self.load_workflow(name) {
            Ok(wf) => wf,
            Err(reply) => return reply,
        };
        if let Err(reply) = self
            .init_and_persist_run(&workflow, name, &ctx.session_id)
            .await
        {
            return reply;
        }
        self.inject_workflow_context(&workflow, &ctx.session_id)
            .await;
        if let Err(reply) = self
            .push_goal_message(&workflow, name, &ctx.session_id)
            .await
        {
            return reply;
        }
        SlashResult::Reply(format!(
            "工作流 \"{name}\" 已启动。正在执行 Step 0: {}",
            workflow.steps[0].name,
        ))
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
}

impl WorkflowSlashHandler {
    /// Load workflow definition via three-level lookup.
    fn load_workflow(&self, name: &str) -> Result<Workflow, SlashResult> {
        WorkflowDefinitionLoader::load(
            name,
            self.agent_workspace.as_deref(),
            self.dot_closeclaw.as_deref(),
        )
        .map_err(|e| SlashResult::Reply(format!("工作流 \"{name}\" 加载失败：{e}")))
    }

    /// Initialize WorkflowRun and persist to checkpoint.
    async fn init_and_persist_run(
        &self,
        workflow: &Workflow,
        name: &str,
        session_id: &str,
    ) -> Result<(), SlashResult> {
        let run = WorkflowEngine::start(workflow);
        self.session_manager
            .set_workflow_run(session_id, Some(Box::new(run)))
            .await
            .map_err(|e| {
                SlashResult::Reply(format!("工作流 \"{name}\" 启动失败（持久化错误）：{e}"))
            })
    }

    /// Inject workflow context into system_appends.
    async fn inject_workflow_context(&self, workflow: &Workflow, session_id: &str) {
        let context = Self::build_workflow_context_append(workflow);
        self.session_manager
            .add_system_append(session_id, context)
            .await;
    }

    /// Push Step 0 goal message as pending.
    async fn push_goal_message(
        &self,
        workflow: &Workflow,
        name: &str,
        session_id: &str,
    ) -> Result<(), SlashResult> {
        let goal = Self::build_goal_message(workflow);
        let pending_msg = PendingMessage::with_role(
            format!("workflow-goal-{}", session_id),
            goal,
            "workflow".to_string(),
        );
        self.session_manager
            .push_pending_message(session_id, pending_msg)
            .await
            .map_err(|e| {
                SlashResult::Reply(format!("工作流 \"{name}\" 启动失败（消息注入错误）：{e}"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_workflow::definition::{Step, Workflow};

    fn make_test_workflow() -> Workflow {
        Workflow {
            id: "test-wf".to_string(),
            name: "Test Workflow".to_string(),
            description: "A test workflow".to_string(),
            version: Some("0.1".to_string()),
            allow_blocked: false,
            verify_retry_limit: 3,
            step_data_schema: serde_yaml::Value::Null,
            steps: vec![Step {
                id: 0,
                name: "Step Zero".to_string(),
                allow_blocked: None,
                goal: "Do the first thing".to_string(),
                verify: vec![],
                jump: vec![],
                transitions: vec![],
            }],
        }
    }

    #[test]
    fn test_build_workflow_context_append() {
        let wf = make_test_workflow();
        let ctx = WorkflowSlashHandler::build_workflow_context_append(&wf);
        assert!(ctx.starts_with("--- WORKFLOW ---"));
        assert!(ctx.ends_with("--- WORKFLOW END ---"));
        assert!(ctx.contains("Test Workflow"));
        assert!(ctx.contains("A test workflow"));
        assert!(ctx.contains("workflow_verify"));
        assert!(ctx.contains("workflow_jump"));
    }

    #[test]
    fn test_build_goal_message() {
        let wf = make_test_workflow();
        let goal = WorkflowSlashHandler::build_goal_message(&wf);
        assert!(goal.contains("[workflow goal]"));
        assert!(goal.contains("Step 0"));
        assert!(goal.contains("Step Zero"));
        assert!(goal.contains("Do the first thing"));
    }
}
