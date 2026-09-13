//! Workflow handler lifecycle and verify-message transcript management.
//!
//! Extracted from `workflow.rs` to keep `impl ConversationSession` blocks
//! under the CONTRIBUTING.md 100-line cap.

use closeclaw_common::processor::ContentBlock;
use closeclaw_workflow::definition::build_goal_message;
use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;
use closeclaw_workflow::run::Phase;

use super::ConversationSession;

/// Prefix used by [`closeclaw_workflow::definition::build_verify_message`]
/// to render verify messages. Used by
/// [`ConversationSession::remove_workflow_verify_messages`] to distinguish
/// verify messages from goal/recovered messages in the transcript.
pub const VERIFY_MESSAGE_PREFIX: &str = "Verify Step";

/// Prefix used by [`closeclaw_workflow::definition::build_jump_message`]
/// to render jump messages. Used by
/// [`ConversationSession::remove_workflow_jump_messages`] to distinguish
/// jump messages from goal/recovered messages in the transcript.
pub const JUMP_MESSAGE_PREFIX: &str = "Jump Step";

/// Handler lifecycle and verify-message transcript cleanup.
impl ConversationSession {
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

    /// Remove only verify messages from the transcript.
    ///
    /// A verify message is identified by `role == "workflow"` and text
    /// content starting with [`VERIFY_MESSAGE_PREFIX`]. Goal and
    /// recovered messages are preserved.
    pub fn remove_workflow_verify_messages(&mut self) {
        self.remove_workflow_messages_with_prefix(VERIFY_MESSAGE_PREFIX);
    }

    /// Remove only jump messages from the transcript.
    ///
    /// A jump message is identified by `role == "workflow"` and text
    /// content starting with [`JUMP_MESSAGE_PREFIX`]. Goal and
    /// recovered messages are preserved.
    pub fn remove_workflow_jump_messages(&mut self) {
        self.remove_workflow_messages_with_prefix(JUMP_MESSAGE_PREFIX);
    }

    /// Internal helper: remove workflow messages whose first text block
    /// starts with the given prefix.
    fn remove_workflow_messages_with_prefix(&mut self, prefix: &str) {
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
            !text.starts_with(prefix)
        });
        let removed = before - self.messages.len();
        if removed > 0 {
            tracing::debug!(removed, prefix, "removed workflow messages from transcript");
        }
    }
}

/// Transcript cleanup and post-jump phase dispatch.
impl ConversationSession {
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

    /// Dispatch post-jump phase transitions: inject goal for Executing
    /// or trigger exit cleanup for Complete.
    pub(crate) fn dispatch_post_jump_phase(
        &mut self,
        phase: Phase,
        step: usize,
        hint: closeclaw_workflow::run::GoalHint,
    ) {
        match phase {
            Phase::Executing => {
                let goal_msg = {
                    let h = self.workflow_handler.as_ref().unwrap();
                    h.definition()
                        .steps
                        .get(step)
                        .map(|s| build_goal_message(s, hint))
                };
                if let Some(msg) = goal_msg {
                    self.inject_workflow_message(&msg);
                    if let Some(ref mut h) = self.workflow_handler {
                        h.on_goal_injected();
                        self.workflow_run = Some(h.run().clone());
                    }
                    tracing::debug!(step, "goal message injected after jump");
                }
            }
            Phase::Complete => {
                tracing::info!("workflow complete after jump, triggering exit cleanup");
                self.workflow_run = Some(self.workflow_handler.as_ref().unwrap().run().clone());
                let session = self.clone();
                tokio::spawn(async move {
                    let mut session = session;
                    session.cleanup_workflow_exit().await;
                });
            }
            _ => {
                tracing::debug!(
                    phase = ?phase,
                    "jump completed with non-actionable phase"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::llm_session::ConversationSession;
    use closeclaw_common::ContentBlock;
    use closeclaw_workflow::definition::{Step, Workflow};
    use closeclaw_workflow::run::{GoalHint, Phase, WorkflowRun};
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
            current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
            step_history: vec![],
            step_data: serde_yaml::Value::Null,
            pending_goal_hint: GoalHint::default(),
            pending_verify: closeclaw_workflow::run::PendingVerify::default(),
            paused_reason: String::new(),
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
            "      - Check output\n",
            "    transitions:\n",
            "      - action: complete",
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

    // ── remove_workflow_jump_messages ───────────────────────────

    #[test]
    fn test_remove_jump_messages_removes_jump() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
        session.push_message("user", vec![ContentBlock::Text("hello".to_string())]);
        session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast\n  B: slow");
        session.push_message("assistant", vec![ContentBlock::Text("done".to_string())]);

        assert_eq!(session.messages.len(), 4);

        session.remove_workflow_jump_messages();

        // Goal, user, assistant remain; jump removed.
        assert_eq!(session.messages.len(), 3);
        let roles: Vec<&str> = session.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["workflow", "user", "assistant"]);
    }

    #[test]
    fn test_remove_jump_messages_preserves_goal_and_verify() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
        session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
        session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");

        session.remove_workflow_jump_messages();

        // Goal and verify remain; jump removed.
        assert_eq!(session.messages.len(), 2);
        let texts: Vec<String> = session
            .messages
            .iter()
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
        assert!(texts[0].starts_with("[workflow goal]"));
        assert!(texts[1].starts_with("Verify Step"));
    }

    #[test]
    fn test_remove_jump_messages_empty_transcript() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.remove_workflow_jump_messages();
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_remove_jump_messages_only_removes_role_workflow() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        // Non-workflow message with same prefix text — should NOT be removed.
        session.push_message(
            "assistant",
            vec![ContentBlock::Text(
                "Jump Step 0 (Step 0):\nQ1\n  A: fast".to_string(),
            )],
        );
        session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast\n  B: slow");

        session.remove_workflow_jump_messages();

        // Assistant message preserved, workflow jump removed.
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, "assistant");
    }
}
