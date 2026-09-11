//! Workflow handler lifecycle and verify-message transcript management.
//!
//! Extracted from `workflow.rs` to keep `impl ConversationSession` blocks
//! under the CONTRIBUTING.md 100-line cap.

use closeclaw_common::processor::ContentBlock;
use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;

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
    /// Remove tool exchange messages (tool_call + tool_result) from
    /// assistant messages in the transcript.
    ///
    /// Scans every assistant message for `ContentBlock::ToolUse` blocks
    /// whose `name` is contained in `tool_names`, removes those blocks,
    /// and also removes any `ContentBlock::ToolResult` blocks whose
    /// `tool_call_id` matches a removed ToolUse's `id`.
    ///
    /// This implements the design doc requirement that verify/jump phases
    /// must "抹除注入消息 + tool_call + tool_result 三条消息" after the
    /// corresponding tool call completes.
    pub fn remove_workflow_tool_exchange(&mut self, tool_names: &[&str]) {
        let mut removed_ids = std::collections::HashSet::new();

        // First pass: scan ALL messages for ToolUse ids to remove.
        for msg in &self.messages {
            for block in &msg.content_blocks {
                if let ContentBlock::ToolUse { id, name, .. } = block {
                    if tool_names.contains(&name.as_str()) {
                        removed_ids.insert(id.clone());
                    }
                }
            }
        }

        if removed_ids.is_empty() {
            return;
        }

        let mut total_removed = 0usize;

        // Second pass: remove matching ToolUse and ToolResult blocks
        // from all messages.
        for msg in &mut self.messages {
            let before = msg.content_blocks.len();
            msg.content_blocks.retain(|block| match block {
                ContentBlock::ToolUse { id, name, .. } => {
                    !(tool_names.contains(&name.as_str()) && removed_ids.contains(id))
                }
                ContentBlock::ToolResult { tool_call_id, .. } => {
                    !removed_ids.contains(tool_call_id)
                }
                _ => true,
            });
            total_removed += before - msg.content_blocks.len();
        }

        // Remove messages that became empty after cleanup.
        let before = self.messages.len();
        self.messages.retain(|m| !m.content_blocks.is_empty());
        total_removed += before - self.messages.len();

        if total_removed > 0 {
            tracing::debug!(
                total_removed,
                ?tool_names,
                removed_ids = removed_ids.len(),
                "removed tool exchange blocks from transcript"
            );
        }
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

    // ── remove_workflow_tool_exchange ────────────────────────────

    #[test]
    fn test_remove_tool_exchange_removes_verify_tool_call_and_result() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        // Setup: goal → verify injected → assistant calls workflow_verify → tool_result.
        session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
        session.push_message(
            "assistant",
            vec![
                ContentBlock::Text("I think it's done.".to_string()),
                ContentBlock::ToolUse {
                    id: "call_v1".to_string(),
                    name: "workflow_verify".to_string(),
                    input: r#"{"result": "pass"}"#.to_string(),
                },
            ],
        );
        session.push_message(
            "user",
            vec![ContentBlock::ToolResult {
                tool_call_id: "call_v1".to_string(),
                content: r#"{"status": "ok"}"#.to_string(),
            }],
        );
        // Unrelated user message should survive.
        session.push_message("user", vec![ContentBlock::Text("hello".to_string())]);

        assert_eq!(session.messages.len(), 4);

        session.remove_workflow_tool_exchange(&["workflow_verify", "workflow_blocked"]);

        // Verify injected message preserved (workflow role, not tool exchange).
        // Assistant ToolUse removed, ToolResult removed.
        // Remaining: verify workflow, assistant (text only), user "hello".
        assert_eq!(session.messages.len(), 3);
        assert_eq!(session.messages[0].role, "workflow");
        assert_eq!(session.messages[1].role, "assistant");
        assert_eq!(session.messages[1].content_blocks.len(), 1);
        assert!(
            matches!(&session.messages[1].content_blocks[0], ContentBlock::Text(t) if t == "I think it's done.")
        );
        assert_eq!(session.messages[2].role, "user");
    }

    #[test]
    fn test_remove_tool_exchange_removes_jump_tool_call_and_result() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        // Setup: jump injected → assistant calls workflow_jump → tool_result.
        session.inject_workflow_message("Jump Step 0 (Step 0):\nQ1\n  A: fast");
        session.push_message(
            "assistant",
            vec![ContentBlock::ToolUse {
                id: "call_j1".to_string(),
                name: "workflow_jump".to_string(),
                input: r#"{"answers": ["A"]}"#.to_string(),
            }],
        );
        session.push_message(
            "user",
            vec![ContentBlock::ToolResult {
                tool_call_id: "call_j1".to_string(),
                content: r#"{"action": "goto", "target": 1}"#.to_string(),
            }],
        );
        session.push_message(
            "user",
            vec![ContentBlock::Text("next question".to_string())],
        );

        assert_eq!(session.messages.len(), 4);

        session.remove_workflow_tool_exchange(&["workflow_jump"]);

        // Jump injected message preserved (workflow role, not tool exchange).
        // Assistant ToolUse removed, ToolResult removed.
        // Remaining: jump workflow, user "next question".
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, "workflow");
        assert_eq!(session.messages[1].role, "user");
    }

    #[test]
    fn test_remove_tool_exchange_preserves_non_matching_tools() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        // Assistant calls a different tool — should not be touched.
        session.push_message(
            "assistant",
            vec![ContentBlock::ToolUse {
                id: "call_read".to_string(),
                name: "read_file".to_string(),
                input: r#"{"path": "/tmp/foo"}"#.to_string(),
            }],
        );
        session.push_message(
            "user",
            vec![ContentBlock::ToolResult {
                tool_call_id: "call_read".to_string(),
                content: "file contents".to_string(),
            }],
        );

        session.remove_workflow_tool_exchange(&["workflow_verify"]);

        // Nothing removed — tool name doesn't match.
        assert_eq!(session.messages.len(), 2);
    }

    #[test]
    fn test_remove_tool_exchange_empty_transcript() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.remove_workflow_tool_exchange(&["workflow_verify"]);
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_remove_tool_exchange_removes_empty_assistant_after_cleanup() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        // Assistant message with only a workflow_verify ToolUse — after removal
        // the message becomes empty and should be dropped.
        session.push_message(
            "assistant",
            vec![ContentBlock::ToolUse {
                id: "call_only".to_string(),
                name: "workflow_verify".to_string(),
                input: r#"{"result": "pass"}"#.to_string(),
            }],
        );
        session.push_message(
            "user",
            vec![ContentBlock::ToolResult {
                tool_call_id: "call_only".to_string(),
                content: r#"{"status": "ok"}"#.to_string(),
            }],
        );

        session.remove_workflow_tool_exchange(&["workflow_verify"]);

        // Both messages removed: assistant becomes empty then dropped,
        // ToolResult has no matching ToolUse remaining.
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_remove_tool_exchange_preserves_goal_in_transcript() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.inject_workflow_message("[workflow goal] Step 0: Do thing");
        session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
        session.push_message(
            "assistant",
            vec![
                ContentBlock::Text("done".to_string()),
                ContentBlock::ToolUse {
                    id: "call_v2".to_string(),
                    name: "workflow_verify".to_string(),
                    input: r#"{"result": "pass"}"#.to_string(),
                },
            ],
        );
        session.push_message(
            "user",
            vec![ContentBlock::ToolResult {
                tool_call_id: "call_v2".to_string(),
                content: r#"{"status": "ok"}"#.to_string(),
            }],
        );

        assert_eq!(session.messages.len(), 4);

        session.remove_workflow_tool_exchange(&["workflow_verify"]);

        // Goal and verify messages preserved (workflow role, not assistant).
        // Assistant message retains Text block but loses ToolUse.
        // ToolResult removed.
        assert_eq!(session.messages.len(), 3);
        // First message: goal workflow
        assert_eq!(session.messages[0].role, "workflow");
        // Second message: verify workflow
        assert_eq!(session.messages[1].role, "workflow");
        // Third message: assistant with only Text block
        assert_eq!(session.messages[2].role, "assistant");
        assert_eq!(session.messages[2].content_blocks.len(), 1);
        assert!(
            matches!(&session.messages[2].content_blocks[0], ContentBlock::Text(t) if t == "done")
        );
    }
}
