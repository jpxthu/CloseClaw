//! Workflow-related methods for `ConversationSession`.

use closeclaw_common::processor::ContentBlock;
use closeclaw_workflow::definition::{build_goal_message, build_jump_message};
use closeclaw_workflow::run::Phase;

use crate::workflow_handler::JumpResult;

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
    ///
    /// Returns `true` if any action was processed.
    /// If the workflow transitions to jumping phase, removes completed
    /// verify messages and injects a jump message into the transcript.
    pub fn process_workflow_tool_results(&mut self, blocks: &[ContentBlock]) -> bool {
        self.ensure_workflow_handler();
        if let Some(ref mut handler) = self.workflow_handler {
            let was_jumping = handler.run().phase == Phase::Jumping;
            let (processed, jump_result) = handler.process_content_blocks(blocks);
            if processed {
                self.workflow_run = Some(handler.run().clone());
            }
            let jump_msg = if matches!(jump_result, JumpResult::Jumped) {
                let current_step = handler.run().current_step;
                handler
                    .definition()
                    .steps
                    .get(current_step)
                    .map(build_jump_message)
            } else {
                None
            };
            // handler borrow ends here; safe to call self methods
            if let Some(msg) = jump_msg {
                // Verify completed → remove verify messages + tool exchange.
                self.remove_workflow_verify_messages();
                self.remove_workflow_tool_exchange(&["workflow_verify", "workflow_blocked"]);
                self.inject_workflow_message(&msg);
                tracing::debug!("jump message injected into transcript");
            } else if was_jumping {
                // Jump completed → remove jump messages + tool exchange.
                self.remove_workflow_jump_messages();
                self.remove_workflow_tool_exchange(&["workflow_jump"]);
                // Post-jump phase dispatch: inject goal or trigger cleanup.
                let (current_phase, current_step, hint) = {
                    let h = self.workflow_handler.as_ref().unwrap();
                    (
                        h.run().phase.clone(),
                        h.run().current_step,
                        h.run().pending_goal_hint.clone(),
                    )
                };
                match current_phase {
                    Phase::Executing => {
                        // goto or reexecute: inject goal message for new step.
                        let goal_msg = {
                            let h = self.workflow_handler.as_ref().unwrap();
                            h.definition()
                                .steps
                                .get(current_step)
                                .map(|step| build_goal_message(step, hint))
                        };
                        if let Some(msg) = goal_msg {
                            self.inject_workflow_message(&msg);
                            if let Some(ref mut h) = self.workflow_handler {
                                h.on_goal_injected();
                                self.workflow_run = Some(h.run().clone());
                            }
                            tracing::debug!(
                                step = current_step,
                                "goal message injected after jump"
                            );
                        }
                    }
                    Phase::Complete => {
                        // Workflow complete: trigger exit cleanup.
                        tracing::info!("workflow complete after jump, triggering exit cleanup");
                        self.workflow_run =
                            Some(self.workflow_handler.as_ref().unwrap().run().clone());
                        let session = self.clone();
                        tokio::spawn(async move {
                            let mut session = session;
                            session.cleanup_workflow_exit().await;
                        });
                    }
                    _ => {
                        tracing::debug!(
                            phase = ?current_phase,
                            "jump completed with non-actionable phase"
                        );
                    }
                }
                tracing::debug!("jump messages cleaned up after phase transition");
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
    /// from system_injection_appends.
    pub fn remove_workflow_context_from_appends(&mut self) {
        let before = self.system_injection_appends.len();
        self.system_injection_appends
            .retain(|s| !s.starts_with("--- WORKFLOW ---"));
        let removed = before - self.system_injection_appends.len();
        if removed > 0 {
            tracing::debug!(
                removed,
                "removed workflow context from system_injection_appends"
            );
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
    use closeclaw_workflow::run::{GoalHint, Phase, WorkflowRun};

    fn make_test_run(definition_name: &str) -> WorkflowRun {
        WorkflowRun {
            workflow_id: "test-wf".to_string(),
            definition_name: definition_name.to_string(),
            definition_version: "0.1".to_string(),
            current_step: 0,
            phase: Phase::Executing,
            step_history: vec![],
            step_data: serde_yaml::Value::Null,
            pending_goal_hint: GoalHint::default(),
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

    /// Write a two-step workflow SKILL.md with a default goto transition.
    fn write_two_step_skill_md(dir: &std::path::Path, workflow_name: &str) {
        let wf_dir = dir.join("workflows").join(workflow_name);
        std::fs::create_dir_all(&wf_dir).unwrap();
        let yaml = concat!(
            "id: test-wf\n",
            "name: Test WF\n",
            "description: test\n",
            "steps:\n",
            "  - id: 0\n",
            "    name: Step 0\n",
            "    goal: Do first thing\n",
            "    transitions:\n",
            "      - action: goto\n",
            "        target_step: 1\n",
            "  - id: 1\n",
            "    name: Step 1\n",
            "    goal: Do second thing\n",
        );
        let content = format!("---\n{yaml}\n---\n\nBody.\n");
        std::fs::write(wf_dir.join("SKILL.md"), content).unwrap();
    }

    /// Write a single-step workflow SKILL.md with a default complete transition.
    fn write_complete_skill_md(dir: &std::path::Path, workflow_name: &str) {
        let wf_dir = dir.join("workflows").join(workflow_name);
        std::fs::create_dir_all(&wf_dir).unwrap();
        let yaml = concat!(
            "id: test-wf\n",
            "name: Test WF\n",
            "description: test\n",
            "steps:\n",
            "  - id: 0\n",
            "    name: Step 0\n",
            "    goal: Do thing\n",
            "    transitions:\n",
            "      - action: complete\n",
        );
        let content = format!("---\n{yaml}\n---\n\nBody.\n");
        std::fs::write(wf_dir.join("SKILL.md"), content).unwrap();
    }

    /// Write a single-step workflow SKILL.md with a default reexecute transition.
    fn write_reexecute_skill_md(dir: &std::path::Path, workflow_name: &str) {
        let wf_dir = dir.join("workflows").join(workflow_name);
        std::fs::create_dir_all(&wf_dir).unwrap();
        let yaml = concat!(
            "id: test-wf\n",
            "name: Test WF\n",
            "description: test\n",
            "steps:\n",
            "  - id: 0\n",
            "    name: Step 0\n",
            "    goal: Do thing\n",
            "    transitions:\n",
            "      - action: reexecute\n",
            "        target_step: 0\n",
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
            pending_goal_hint: GoalHint::default(),
            pending_verify: 0,
        };
        let mut handler = WorkflowHandler::new(run, definition);

        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "c1".to_string(),
            content: r#"{"action": "workflow_blocked", "reason": "test"}"#.to_string(),
        }];
        assert!(handler.process_content_blocks(&blocks).0);
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
        let (processed, _) = session
            .workflow_handler_mut()
            .unwrap()
            .process_content_blocks(&blocks);
        assert!(processed, "direct handler call should work");
        assert_eq!(
            session.workflow_handler().unwrap().run().phase,
            Phase::Blocked
        );
    }

    // ── Step 1.3: jump → goal injection / complete exit ─────────

    /// After a goto jump, the new step's goal message is injected
    /// into the transcript with Normal hint.
    #[test]
    fn test_goto_jump_injects_goal_message() {
        let tmp = tempfile::tempdir().unwrap();
        write_two_step_skill_md(tmp.path(), "Test WF");

        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            tmp.path().to_path_buf(),
        );
        let mut run = make_test_run("Test WF");
        run.phase = Phase::Jumping; // Simulate jumping phase
        session.set_workflow_run(Some(run));
        session.ensure_workflow_handler();

        // Inject a jump message and an assistant tool_call for workflow_jump.
        session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
        session.push_message(
            "assistant",
            vec![ContentBlock::ToolUse {
                id: "tc1".to_string(),
                name: "workflow_jump".to_string(),
                input: r#"{"answers": {}}"#.to_string(),
            }],
        );
        // Inject tool result with empty answers (triggers default goto).
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "tc1".to_string(),
            content: r#"{"action": "workflow_jump", "answers": {}}"#.to_string(),
        }];

        session.process_workflow_tool_results(&blocks);

        // Jump messages should be removed, goal message injected.
        let wf_messages: Vec<String> = session
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
        assert_eq!(
            wf_messages.len(),
            1,
            "should have exactly one workflow message (goal)"
        );
        assert!(
            wf_messages[0].starts_with("[workflow goal]"),
            "workflow message should be a goal: {}",
            wf_messages[0]
        );
        assert!(
            wf_messages[0].contains("Step 1"),
            "goal should reference step 1: {}",
            wf_messages[0]
        );
        // pending_goal_hint should be consumed (reset to Normal).
        let handler = session.workflow_handler().unwrap();
        assert_eq!(handler.run().pending_goal_hint, GoalHint::Normal);
        assert_eq!(handler.run().current_step, 1);
    }

    /// After a reexecute jump, the goal message includes the reexecute hint.
    #[test]
    fn test_reexecute_jump_injects_goal_with_hint() {
        let tmp = tempfile::tempdir().unwrap();
        write_reexecute_skill_md(tmp.path(), "Test WF");

        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            tmp.path().to_path_buf(),
        );
        let mut run = make_test_run("Test WF");
        run.phase = Phase::Jumping;
        session.set_workflow_run(Some(run));
        session.ensure_workflow_handler();

        session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
        session.push_message(
            "assistant",
            vec![ContentBlock::ToolUse {
                id: "tc1".to_string(),
                name: "workflow_jump".to_string(),
                input: r#"{"answers": {}}"#.to_string(),
            }],
        );
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "tc1".to_string(),
            content: r#"{"action": "workflow_jump", "answers": {}}"#.to_string(),
        }];

        session.process_workflow_tool_results(&blocks);

        let wf_messages: Vec<String> = session
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
        assert_eq!(wf_messages.len(), 1);
        assert!(wf_messages[0].starts_with("[workflow goal]"));
        assert!(
            wf_messages[0].contains("重新执行"),
            "reexecute goal should contain reexecute hint: {}",
            wf_messages[0]
        );
        let handler = session.workflow_handler().unwrap();
        assert_eq!(handler.run().pending_goal_hint, GoalHint::Normal);
        assert_eq!(handler.run().current_step, 0);
    }

    /// After a complete jump, workflow_run is set and handler is cleared.
    #[tokio::test]
    async fn test_complete_jump_triggers_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        write_complete_skill_md(tmp.path(), "Test WF");

        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            tmp.path().to_path_buf(),
        );
        let mut run = make_test_run("Test WF");
        run.phase = Phase::Jumping;
        session.set_workflow_run(Some(run));
        session.ensure_workflow_handler();

        session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
        session.push_message(
            "assistant",
            vec![ContentBlock::ToolUse {
                id: "tc1".to_string(),
                name: "workflow_jump".to_string(),
                input: r#"{"answers": {}}"#.to_string(),
            }],
        );
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "tc1".to_string(),
            content: r#"{"action": "workflow_jump", "answers": {}}"#.to_string(),
        }];

        session.process_workflow_tool_results(&blocks);

        // Jump messages removed, no new goal message.
        let wf_messages: Vec<&str> = session
            .messages
            .iter()
            .filter(|m| m.role == "workflow")
            .map(|_m| "workflow")
            .collect();
        assert!(
            wf_messages.is_empty(),
            "no workflow messages should remain after complete"
        );
        // workflow_run should be set (from handler.clone()).
        assert!(session.workflow_run().is_some());
        assert_eq!(session.workflow_run().unwrap().phase, Phase::Complete);
    }

    /// After goal injection via pending_goal_hint, hint resets to Normal.
    #[test]
    fn test_pending_goal_hint_resets_after_injection() {
        let tmp = tempfile::tempdir().unwrap();
        write_reexecute_skill_md(tmp.path(), "Test WF");

        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            tmp.path().to_path_buf(),
        );
        let mut run = make_test_run("Test WF");
        run.phase = Phase::Jumping;
        run.pending_goal_hint = GoalHint::Reexecute; // Set hint before jump
        session.set_workflow_run(Some(run));
        session.ensure_workflow_handler();

        session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
        session.push_message(
            "assistant",
            vec![ContentBlock::ToolUse {
                id: "tc1".to_string(),
                name: "workflow_jump".to_string(),
                input: r#"{"answers": {}}"#.to_string(),
            }],
        );
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "tc1".to_string(),
            content: r#"{"action": "workflow_jump", "answers": {}}"#.to_string(),
        }];

        session.process_workflow_tool_results(&blocks);

        // Handler hint should be Normal after injection consumed it.
        let handler = session.workflow_handler().unwrap();
        assert_eq!(handler.run().pending_goal_hint, GoalHint::Normal);
    }
}
