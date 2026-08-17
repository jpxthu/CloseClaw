//! Workflow-related methods for `ConversationSession`.

use closeclaw_common::processor::ContentBlock;
use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;

use super::ConversationSession;

/// Prefix used by [`closeclaw_workflow::definition::build_verify_message`]
/// to render verify messages. Used by [`ConversationSession::remove_workflow_verify_messages`]
/// to distinguish verify messages from goal/recovered messages in the transcript.
pub const VERIFY_MESSAGE_PREFIX: &str = "Verify Step";

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

    /// Lazily build the [`WorkflowHandler`] if a `workflow_run` exists
    /// but no handler is present.
    ///
    /// Loads the workflow definition via the three-level priority lookup:
    /// 1. `{agent_workspace}/workflows/{name}/SKILL.md`
    /// 2. `{dot_closeclaw}/workflows/{name}/SKILL.md`
    /// 3. Built-in (currently a no-op placeholder)
    ///
    /// On failure, logs a warning and leaves the handler as `None`.
    /// Does not panic and does not block the session.
    pub fn ensure_workflow_handler(&mut self) {
        if self.workflow_handler.is_some() {
            return;
        }
        let run = match self.workflow_run.clone() {
            Some(r) => r,
            None => return,
        };
        let dot_closeclaw = self.workdir.join(".closeclaw");
        let definition = match WorkflowDefinitionLoader::load(
            &run.definition_name,
            Some(self.workdir.as_path()),
            Some(dot_closeclaw.as_path()),
        ) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    workflow = %run.definition_name,
                    error = %e,
                    "failed to load workflow definition, handler remains None"
                );
                return;
            }
        };
        self.workflow_handler = Some(crate::workflow_handler::WorkflowHandler::new(
            run, definition,
        ));
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

    /// Remove only verify messages from the transcript.
    ///
    /// A verify message is identified by `role == "workflow"` and text
    /// content starting with [`VERIFY_MESSAGE_PREFIX`] (the output of
    /// [`closeclaw_workflow::definition::build_verify_message`]).
    /// Goal and recovered messages are preserved.
    pub fn remove_workflow_verify_messages(&mut self) {
        let before = self.messages.len();
        self.messages.retain(|m| {
            if m.role != "workflow" {
                return true;
            }
            let text = m
                .content_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .next()
                .unwrap_or("");
            !text.starts_with(VERIFY_MESSAGE_PREFIX)
        });
        let removed = before - self.messages.len();
        if removed > 0 {
            tracing::debug!(removed, "removed workflow verify messages from transcript");
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
    use std::path::PathBuf;

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
                name: "Step 0".to_string(),
                goal: "Do first thing".to_string(),
                verify: vec!["Check output".to_string()],
                jump: vec![],
                transitions: vec![],
                allow_blocked: Some(true),
            }],
        }
    }

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

    // ── ensure_workflow_handler ──────────────────────────────────

    #[test]
    fn test_ensure_workflow_handler_success() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill_md(tmp.path(), "Test Workflow");

        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            tmp.path().to_path_buf(),
        );
        session.set_workflow_run(Some(make_test_run("Test Workflow")));
        assert!(session.workflow_handler().is_none());

        session.ensure_workflow_handler();

        let handler = session.workflow_handler().expect("handler should be set");
        assert_eq!(handler.definition().id, "test-wf");
        assert_eq!(handler.run().definition_name, "Test Workflow");
    }

    #[test]
    fn test_ensure_workflow_handler_no_run() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        // No workflow_run set.
        session.ensure_workflow_handler();
        assert!(session.workflow_handler().is_none());
    }

    #[test]
    fn test_ensure_workflow_handler_already_set() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill_md(tmp.path(), "Test Workflow");

        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            tmp.path().to_path_buf(),
        );
        session.set_workflow_run(Some(make_test_run("Test Workflow")));
        // Pre-set a handler — ensure_workflow_handler should not overwrite.
        let existing = crate::workflow_handler::WorkflowHandler::new(
            make_test_run("Test Workflow"),
            make_test_workflow(),
        );
        session.set_workflow_handler(Some(existing));

        session.ensure_workflow_handler();

        // Handler should still be the pre-existing one (same definition).
        let handler = session.workflow_handler().unwrap();
        assert_eq!(handler.definition().id, "test-wf");
    }

    #[test]
    fn test_ensure_workflow_handler_load_failure() {
        let tmp = tempfile::tempdir().unwrap();
        // No SKILL.md written — loader will return DefinitionNotFound.

        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            tmp.path().to_path_buf(),
        );
        session.set_workflow_run(Some(make_test_run("nonexistent")));

        // Should not panic; handler remains None.
        session.ensure_workflow_handler();
        assert!(session.workflow_handler().is_none());
    }

    // ── process_workflow_tool_results with ensure ────────────────

    #[test]
    fn test_process_workflow_tool_results_with_existing_handler() {
        use crate::workflow_handler::WorkflowHandler;
        use closeclaw_workflow::definition::{Step, Workflow};
        use closeclaw_workflow::run::{Phase, WorkflowRun};

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

    // ── remove_workflow_verify_messages ──────────────────────────

    #[test]
    fn test_remove_verify_messages_preserves_goal_and_recovered() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
        session.push_message("user", vec![ContentBlock::Text("hello".to_string())]);
        session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
        session.inject_workflow_message("recovered: resuming after crash");
        session.push_message("assistant", vec![ContentBlock::Text("done".to_string())]);

        assert_eq!(session.messages.len(), 5);

        session.remove_workflow_verify_messages();

        // Goal, user, recovered, assistant remain; verify removed.
        assert_eq!(session.messages.len(), 4);
        let roles: Vec<&str> = session.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["workflow", "user", "workflow", "assistant"]);

        // Verify the remaining workflow messages are goal and recovered.
        let wf_texts: Vec<String> = session
            .messages
            .iter()
            .filter(|m| m.role == "workflow")
            .map(|m| {
                m.content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .next()
                    .unwrap_or_default()
            })
            .collect();
        assert!(wf_texts[0].starts_with("[workflow goal]"));
        assert!(wf_texts[1].starts_with("recovered"));
    }

    #[test]
    fn test_remove_verify_messages_no_verify() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo thing");
        session.push_message("user", vec![ContentBlock::Text("hi".to_string())]);

        session.remove_workflow_verify_messages();

        assert_eq!(session.messages.len(), 2);
    }

    #[test]
    fn test_remove_verify_messages_only_removes_role_workflow() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        // Non-workflow message with same prefix text — should NOT be removed.
        session.push_message(
            "assistant",
            vec![ContentBlock::Text("Verify Step 0 (Step 0):".to_string())],
        );
        session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");

        session.remove_workflow_verify_messages();

        // Assistant message preserved, workflow verify removed.
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, "assistant");
    }

    #[test]
    fn test_remove_verify_messages_empty_transcript() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.remove_workflow_verify_messages();
        assert!(session.messages.is_empty());
    }
}
