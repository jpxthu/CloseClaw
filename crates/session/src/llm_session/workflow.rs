//! Workflow-related methods for `ConversationSession`.

use closeclaw_common::processor::ContentBlock;

use super::ConversationSession;

/// Workflow methods: run/handler access, tool result processing,
/// transcript cleanup.
impl ConversationSession {
    /// Returns a reference to the active workflow run, if any.
    pub fn workflow_run(&self) -> Option<&closeclaw_workflow::run::WorkflowRun> {
        self.workflow_run.as_ref()
    }

    /// Sets the active workflow run state.
    pub fn set_workflow_run(&mut self, run: Option<closeclaw_workflow::run::WorkflowRun>) {
        self.workflow_run = run;
    }

    pub fn workflow_handler(&self) -> Option<&crate::workflow_handler::WorkflowHandler> {
        self.workflow_handler.as_ref()
    }

    pub fn workflow_handler_mut(
        &mut self,
    ) -> Option<&mut crate::workflow_handler::WorkflowHandler> {
        self.workflow_handler.as_mut()
    }

    pub fn set_workflow_handler(
        &mut self,
        handler: Option<crate::workflow_handler::WorkflowHandler>,
    ) {
        self.workflow_handler = handler;
    }

    /// Process workflow tool results from LLM content blocks.
    /// Returns `true` if any action was processed.
    pub fn process_workflow_tool_results(&mut self, blocks: &[ContentBlock]) -> bool {
        self.ensure_workflow_handler();
        if let Some(ref mut handler) = self.workflow_handler {
            let processed = handler.process_content_blocks(blocks);
            if processed {
                self.workflow_run = Some(handler.run().clone());
            }
            processed
        } else {
            false
        }
    }

    pub fn take_workflow_notification(
        &mut self,
    ) -> Option<crate::workflow_handler::WorkflowNotification> {
        self.workflow_handler
            .as_mut()
            .and_then(|h| h.take_notification())
    }

    pub fn is_workflow_blocked(&self) -> bool {
        self.workflow_handler
            .as_ref()
            .is_some_and(|h| h.is_blocked())
    }

    /// Remove all workflow control messages (role == "workflow")
    /// from the transcript.
    pub fn remove_workflow_messages(&mut self) {
        let before = self.messages.len();
        self.messages.retain(|m| m.role != "workflow");
        let removed = before - self.messages.len();
        if removed > 0 {
            tracing::debug!(removed, "removed workflow control messages from transcript");
        }
    }

    /// Inject a workflow control message (role == "workflow")
    /// into the transcript.
    pub fn inject_workflow_message(&mut self, content: &str) {
        self.push_message("workflow", vec![ContentBlock::Text(content.to_string())]);
    }

    /// Remove workflow context ("--- WORKFLOW ---" items)
    /// from system_appends.
    pub fn remove_workflow_context_from_appends(&mut self) {
        let before = self.system_appends.len();
        self.system_appends
            .retain(|s| !s.starts_with("--- WORKFLOW ---"));
        let removed = before - self.system_appends.len();
        if removed > 0 {
            tracing::debug!(removed, "removed workflow context from system_appends");
        }
    }

    /// Reset workflow_run and handler to None.
    pub fn clear_workflow_run(&mut self) {
        self.workflow_run = None;
        self.workflow_handler = None;
    }
}

#[cfg(test)]
mod tests {
    use crate::llm_session::ConversationSession;
    use closeclaw_common::ContentBlock;
    use closeclaw_workflow::definition::{Step, Workflow};
    use closeclaw_workflow::run::{Phase, WorkflowRun};

    fn make_test_run(definition_name: &str) -> WorkflowRun {
        WorkflowRun {
            workflow_id: "test-wf".to_string(),
            definition_name: definition_name.to_string(),
            definition_version: "0.1".to_string(),
            current_step: 0,
            phase: Phase::Executing,
            step_history: vec![],
            step_data: serde_yaml::Value::Null,
            pending_verify: 0,
        }
    }

    fn write_skill_md(dir: &std::path::Path, workflow_name: &str) {
        let wf_dir = dir.join("workflows").join(workflow_name);
        std::fs::create_dir_all(&wf_dir).unwrap();
        let yaml = concat!(
            "id: test-wf\n",
            "name: Test Workflow\n",
            "description: A test workflow\n",
            "steps:\n",
            "  - id: 0\n",
            "    name: Step 0\n",
            "    goal: Do first thing\n",
            "    allow_blocked: true\n",
            "    verify:\n",
            "      - Check output",
        );
        let content = format!("---\n{yaml}\n---\n\nBody.\n");
        std::fs::write(wf_dir.join("SKILL.md"), content).unwrap();
    }

    // ── process_workflow_tool_results with ensure ────────────────

    #[test]
    fn test_process_workflow_tool_results_with_existing_handler() {
        use crate::workflow_handler::WorkflowHandler;

        let definition = Workflow {
            id: "test-wf".to_string(),
            name: "Test WF".to_string(),
            description: "test".to_string(),
            version: None,
            allow_blocked: false,
            verify_retry_limit: 3,
            step_data_schema: serde_yaml::Value::Null,
            steps: vec![Step {
                id: 0,
                name: "S0".to_string(),
                goal: "g".to_string(),
                verify: vec![],
                jump: vec![],
                transitions: vec![],
                allow_blocked: Some(true),
            }],
        };
        let run = WorkflowRun {
            workflow_id: "wf".to_string(),
            definition_name: "Test WF".to_string(),
            definition_version: "0.1".to_string(),
            current_step: 0,
            phase: Phase::Executing,
            step_history: vec![],
            step_data: serde_yaml::Value::Null,
            pending_verify: 0,
        };
        let mut handler = WorkflowHandler::new(run, definition);

        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "c1".to_string(),
            content: r#"{"action": "workflow_blocked", "reason": "test"}"#.to_string(),
        }];
        assert!(handler.process_content_blocks(&blocks));
        assert_eq!(handler.run().phase, Phase::Blocked);
    }

    #[test]
    fn test_process_workflow_tool_results_via_session() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill_md(tmp.path(), "Test Workflow");

        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            tmp.path().to_path_buf(),
        );
        session.set_workflow_run(Some(make_test_run("Test Workflow")));

        // Build handler first.
        session.ensure_workflow_handler();
        assert!(session.workflow_handler().is_some());

        // Process blocks through handler directly.
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "c1".to_string(),
            content: r#"{"action": "workflow_blocked", "reason": "test"}"#.to_string(),
        }];
        let processed = session
            .workflow_handler_mut()
            .unwrap()
            .process_content_blocks(&blocks);
        assert!(processed, "direct handler call should work");
        assert_eq!(
            session.workflow_handler().unwrap().run().phase,
            Phase::Blocked
        );
    }
}
