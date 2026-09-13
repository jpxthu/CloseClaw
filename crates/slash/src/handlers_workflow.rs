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
/// 5. Inject workflow context into system_injection_appends
/// 6. Push Step 0 goal message as pending
/// 7. Return confirmation
#[derive(Clone)]
pub struct WorkflowSlashHandler {
    session_manager: Arc<dyn SlashSessionQuery>,
    agent_workspace: Option<PathBuf>,
    global_workflows: Option<PathBuf>,
}

impl WorkflowSlashHandler {
    /// Create a new WorkflowHandler.
    pub fn new(
        session_manager: Arc<dyn SlashSessionQuery>,
        agent_workspace: Option<PathBuf>,
        global_workflows: Option<PathBuf>,
    ) -> Self {
        Self {
            session_manager,
            agent_workspace,
            global_workflows,
        }
    }

    /// Build the workflow context string to inject into system_injection_appends.
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

    fn immediate(&self, _cmd: &str, _args: &str) -> bool {
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

        // Enforce one-active-workflow-per-session constraint.
        if let Some(phase) = self
            .session_manager
            .get_active_workflow_run_phase(&ctx.session_id)
            .await
        {
            return SlashResult::Reply(format!(
                "无法启动工作流：当前 Session 已有活跃的工作流运行（阶段：{phase}）。请等待当前工作流完成或由 Owner 终止后再启动新工作流。"
            ));
        }

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
            self.global_workflows.as_deref(),
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

    /// Inject workflow context into system_injection_appends.
    async fn inject_workflow_context(&self, workflow: &Workflow, session_id: &str) {
        let context = Self::build_workflow_context_append(workflow);
        self.session_manager
            .add_system_injection_append(session_id, context)
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

    // ── Mock for handler integration tests ─────────────────────────────

    use std::collections::HashMap;
    use std::sync::Mutex;

    use closeclaw_common::session_lookup::PendingMessage;
    use closeclaw_common::SlashSessionQuery;

    /// Mock state shared across async calls.
    struct MockState {
        active_phases: HashMap<String, Option<String>>,
        set_workflow_run_calls: Vec<(String, bool)>,
        injection_appends: Vec<(String, String)>,
        pending_messages: Vec<(String, String)>,
    }

    struct MockQuery {
        state: Mutex<MockState>,
    }

    impl MockQuery {
        fn new() -> Self {
            Self {
                state: Mutex::new(MockState {
                    active_phases: HashMap::new(),
                    set_workflow_run_calls: Vec::new(),
                    injection_appends: Vec::new(),
                    pending_messages: Vec::new(),
                }),
            }
        }

        /// Set the active workflow phase for a session.
        /// `None` means no active workflow; `Some(phase)` means active.
        fn set_active_phase(&self, session_id: &str, phase: Option<String>) {
            self.state
                .lock()
                .unwrap()
                .active_phases
                .insert(session_id.to_string(), phase);
        }

        fn set_workflow_run_calls(&self) -> Vec<(String, bool)> {
            self.state.lock().unwrap().set_workflow_run_calls.clone()
        }

        fn injection_appends(&self) -> Vec<(String, String)> {
            self.state.lock().unwrap().injection_appends.clone()
        }

        fn pending_messages(&self) -> Vec<(String, String)> {
            self.state.lock().unwrap().pending_messages.clone()
        }
    }

    #[async_trait::async_trait]
    impl SlashSessionQuery for MockQuery {
        async fn get_active_workflow_run_phase(&self, session_id: &str) -> Option<String> {
            self.state
                .lock()
                .unwrap()
                .active_phases
                .get(session_id)
                .cloned()
                .flatten()
        }

        async fn set_workflow_run(
            &self,
            session_id: &str,
            run: Option<Box<dyn std::any::Any + Send + Sync>>,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap()
                .set_workflow_run_calls
                .push((session_id.to_string(), run.is_some()));
            Ok(())
        }

        async fn add_system_injection_append(&self, session_id: &str, content: String) {
            self.state
                .lock()
                .unwrap()
                .injection_appends
                .push((session_id.to_string(), content));
        }

        async fn push_pending_message(
            &self,
            session_id: &str,
            msg: PendingMessage,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap()
                .pending_messages
                .push((session_id.to_string(), msg.content));
            Ok(())
        }

        // ── Unused methods: return defaults ──────────────────────────────

        async fn get_plan_state(&self, _: &str) -> Option<closeclaw_common::PlanState> {
            None
        }
        async fn set_plan_state(&self, _: &str, _: closeclaw_common::PlanState) {}
        async fn trigger_manual_background(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }
        async fn invalidate_static_cache(&self) {}
        async fn rebuild_system_prompt_for_session(&self, _: &str) {}
        async fn add_system_append(&self, _: &str, _: String) {}
        async fn get_model(&self, _: &str) -> Option<String> {
            None
        }
        async fn get_reasoning_level(&self, _: &str) -> Option<String> {
            None
        }
        async fn get_verbosity_level(&self, _: &str) -> Option<String> {
            None
        }
        async fn get_session_mode(&self, _: &str) -> Option<closeclaw_common::SessionMode> {
            None
        }
        async fn get_workdir(&self, _: &str) -> Option<std::path::PathBuf> {
            None
        }
        async fn set_workdir(&self, _: &str, _: std::path::PathBuf) {}
        async fn get_system_appends(&self, _: &str) -> Vec<String> {
            vec![]
        }
        async fn is_llm_busy(&self, _: &str) -> bool {
            false
        }
        async fn get_stats(&self, _: &str) -> Option<(usize, usize, usize, usize)> {
            None
        }
        async fn get_last_cache_break(&self, _: &str) -> Option<String> {
            None
        }
        async fn get_active_child_count(&self, _: &str) -> usize {
            0
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Write a single-step workflow SKILL.md with a complete transition.
    fn write_workflow_file(dir: &std::path::Path, name: &str) {
        let wf_dir = dir.join("workflows").join(name);
        std::fs::create_dir_all(&wf_dir).unwrap();
        let yaml = concat!(
            "id: test-wf\n",
            "name: Test Workflow\n",
            "description: A test workflow\n",
            "steps:\n",
            "  - id: 0\n",
            "    name: Step Zero\n",
            "    goal: Do the first thing\n",
            "    verify:\n",
            "      - Check output\n",
            "    transitions:\n",
            "      - action: complete\n",
        );
        let content = format!("---\n{yaml}\n---\n\nBody.\n");
        std::fs::write(wf_dir.join("SKILL.md"), content).unwrap();
    }

    fn make_slash_context(session_id: &str) -> SlashContext {
        SlashContext {
            command: "workflow".to_string(),
            sender_id: "owner".to_string(),
            session_id: session_id.to_string(),
            channel: "test".to_string(),
        }
    }

    // ── Test 1: Normal path — no active workflow → starts successfully ──

    #[tokio::test]
    async fn test_workflow_starts_when_no_active_workflow() {
        let tmp = tempfile::tempdir().unwrap();
        write_workflow_file(tmp.path(), "Test WF");

        let mock = Arc::new(MockQuery::new());
        // No active workflow set → get_active_workflow_run_phase returns None.
        mock.set_active_phase("s1", None);

        let handler = WorkflowSlashHandler::new(mock.clone(), Some(tmp.path().to_path_buf()), None);
        let ctx = make_slash_context("s1");

        let result = handler.handle("Test WF", &ctx).await;

        match result {
            SlashResult::Reply(msg) => {
                assert!(msg.contains("已启动"), "should confirm start: {msg}");
                assert!(msg.contains("Step Zero"), "should mention step: {msg}");
            }
            _ => panic!("expected Reply for successful start"),
        }

        // Verify side effects.
        let calls = mock.set_workflow_run_calls();
        assert_eq!(calls.len(), 1, "should call set_workflow_run once");
        assert!(calls[0].1, "run should be Some");

        let appends = mock.injection_appends();
        assert_eq!(appends.len(), 1, "should inject workflow context");
        assert!(appends[0].1.contains("--- WORKFLOW ---"));

        let pending = mock.pending_messages();
        assert_eq!(pending.len(), 1, "should push goal message");
        assert!(pending[0].1.contains("[workflow goal]"));
    }

    // ── Test 2: Reject path — active workflow (Executing) → error ────────

    #[tokio::test]
    async fn test_workflow_rejected_when_active_executing() {
        let tmp = tempfile::tempdir().unwrap();
        write_workflow_file(tmp.path(), "Test WF");

        let mock = Arc::new(MockQuery::new());
        mock.set_active_phase("s1", Some("Executing".to_string()));

        let handler = WorkflowSlashHandler::new(mock.clone(), Some(tmp.path().to_path_buf()), None);
        let ctx = make_slash_context("s1");

        let result = handler.handle("Test WF", &ctx).await;

        match result {
            SlashResult::Reply(msg) => {
                assert!(msg.contains("无法启动工作流"), "should reject: {msg}");
                assert!(msg.contains("Executing"), "should mention phase: {msg}");
            }
            _ => panic!("expected Reply rejection"),
        }

        // No side effects should occur.
        assert!(mock.set_workflow_run_calls().is_empty());
        assert!(mock.injection_appends().is_empty());
        assert!(mock.pending_messages().is_empty());
    }

    // ── Test 3: Reject path — active workflow (Blocked) → error ──────────

    #[tokio::test]
    async fn test_workflow_rejected_when_active_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        write_workflow_file(tmp.path(), "Test WF");

        let mock = Arc::new(MockQuery::new());
        mock.set_active_phase("s1", Some("Blocked".to_string()));

        let handler = WorkflowSlashHandler::new(mock.clone(), Some(tmp.path().to_path_buf()), None);
        let ctx = make_slash_context("s1");

        let result = handler.handle("Test WF", &ctx).await;

        match result {
            SlashResult::Reply(msg) => {
                assert!(msg.contains("无法启动工作流"), "should reject: {msg}");
                assert!(msg.contains("Blocked"), "should mention phase: {msg}");
            }
            _ => panic!("expected Reply rejection"),
        }

        assert!(mock.set_workflow_run_calls().is_empty());
    }

    // ── Test 4: Boundary — active workflow but Complete → allowed ─────────

    #[tokio::test]
    async fn test_workflow_starts_when_previous_complete() {
        let tmp = tempfile::tempdir().unwrap();
        write_workflow_file(tmp.path(), "Test WF");

        let mock = Arc::new(MockQuery::new());
        // Phase=None means get_active_workflow_run_phase returns None
        // (Complete phase is filtered out by the real implementation).
        mock.set_active_phase("s1", None);

        let handler = WorkflowSlashHandler::new(mock.clone(), Some(tmp.path().to_path_buf()), None);
        let ctx = make_slash_context("s1");

        let result = handler.handle("Test WF", &ctx).await;

        match result {
            SlashResult::Reply(msg) => {
                assert!(msg.contains("已启动"), "should start after complete: {msg}");
            }
            _ => panic!("expected Reply for start after complete"),
        }

        assert_eq!(mock.set_workflow_run_calls().len(), 1);
        assert_eq!(mock.injection_appends().len(), 1);
        assert_eq!(mock.pending_messages().len(), 1);
    }

    // ── Test 5: Empty name → usage hint ──────────────────────────────────

    #[tokio::test]
    async fn test_workflow_empty_name_returns_usage() {
        let mock = Arc::new(MockQuery::new());
        let handler = WorkflowSlashHandler::new(mock, None, None);
        let ctx = make_slash_context("s1");

        let result = handler.handle("", &ctx).await;

        match result {
            SlashResult::Reply(msg) => {
                assert!(msg.contains("用法"), "should show usage: {msg}");
            }
            _ => panic!("expected Reply for empty name"),
        }
    }

    // ── Test 6: Non-existent workflow → error ────────────────────────────

    #[tokio::test]
    async fn test_workflow_nonexistent_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        // No workflow file written.

        let mock = Arc::new(MockQuery::new());
        let handler = WorkflowSlashHandler::new(mock, Some(tmp.path().to_path_buf()), None);
        let ctx = make_slash_context("s1");

        let result = handler.handle("NonExistent", &ctx).await;

        match result {
            SlashResult::Reply(msg) => {
                assert!(msg.contains("加载失败"), "should report load error: {msg}");
            }
            _ => panic!("expected Reply for load error"),
        }
    }
}
