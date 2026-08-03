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
use closeclaw_gateway::SessionManager;
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
pub struct WorkflowHandler {
    session_manager: Arc<SessionManager>,
    agent_workspace: Option<PathBuf>,
    dot_closeclaw: Option<PathBuf>,
}

impl WorkflowHandler {
    /// Create a new WorkflowHandler.
    pub fn new(
        session_manager: Arc<SessionManager>,
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
    /// Format matches the design doc spec:
    /// ```text
    /// --- WORKFLOW ---
    /// 你正在执行受控工作流：{workflow_name}
    /// 描述：{description}
    /// Engine 会通过 workflow 角色消息驱动步骤推进，必须遵守三阶段协议：
    /// 1. 收到 goal → 执行步骤
    /// 2. 收到 verify → 自查验收清单 → 完成则调用 workflow_verify，否则继续
    /// 3. 收到 jump → 回答问题 → 调用 workflow_jump 传递答案
    /// 不要自行跳步或跳过验证。
    /// --- WORKFLOW END ---
    /// ```
    pub fn build_workflow_context_append(workflow: &Workflow) -> String {
        format!(
            "--- WORKFLOW ---\n\
             你正在执行受控工作流：{name}\n\
             描述：{desc}\n\
             Engine 会通过 workflow 角色消息驱动步骤推进，必须遵守三阶段协议：\n\
             1. 收到 goal → 执行步骤\n\
             2. 收到 verify → 自查验收清单 → 完成则调用 workflow_verify，否则继续\n\
             3. 收到 jump → 回答问题 → 调用 workflow_jump 传递答案\n\
             不要自行跳步或跳过验证。\n\
             --- WORKFLOW END ---",
            name = workflow.name,
            desc = workflow.description,
        )
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
impl SlashHandler for WorkflowHandler {
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

        // 1. Load workflow definition via three-level lookup.
        let workflow: Workflow = match WorkflowDefinitionLoader::load(
            name,
            self.agent_workspace.as_deref(),
            self.dot_closeclaw.as_deref(),
        ) {
            Ok(wf) => wf,
            Err(e) => {
                return SlashResult::Reply(format!("工作流 \"{name}\" 加载失败：{e}"));
            }
        };

        // 2. Initialize WorkflowRun.
        let run = WorkflowEngine::start(&workflow);

        // 3. Persist WorkflowRun to session checkpoint.
        if let Err(e) = self
            .session_manager
            .set_workflow_run(&ctx.session_id, Some(run))
            .await
        {
            return SlashResult::Reply(format!("工作流 \"{name}\" 启动失败（持久化错误）：{e}"));
        }

        // 4. Inject workflow context into system_appends.
        let context = Self::build_workflow_context_append(&workflow);
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        {
            let mut cs = cs.write().await;
            cs.add_system_append(context);
        }

        // 5. Push Step 0 goal message as pending.
        let goal = Self::build_goal_message(&workflow);
        let pending_msg = PendingMessage::with_role(
            format!("workflow-goal-{}", ctx.session_id),
            goal,
            "workflow".to_string(),
        );
        if let Err(e) = self
            .session_manager
            .push_pending_message(&ctx.session_id, pending_msg)
            .await
        {
            return SlashResult::Reply(format!("工作流 \"{name}\" 启动失败（消息注入错误）：{e}"));
        }

        // 6. Return confirmation.
        SlashResult::Reply(format!(
            "工作流 \"{name}\" 已启动。正在执行 Step 0: {}",
            workflow.steps[0].name,
        ))
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
        let ctx = WorkflowHandler::build_workflow_context_append(&wf);
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
        let goal = WorkflowHandler::build_goal_message(&wf);
        assert!(goal.contains("[workflow goal]"));
        assert!(goal.contains("Step 0"));
        assert!(goal.contains("Step Zero"));
        assert!(goal.contains("Do the first thing"));
    }
}
