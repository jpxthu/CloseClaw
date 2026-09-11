//! Tool-exchange cleanup for workflow transcripts.
//!
//! Extracted from `workflow_lifecycle.rs` to keep `impl ConversationSession`
//! blocks under the CONTRIBUTING.md 100-line cap.

use closeclaw_common::processor::ContentBlock;

use super::ConversationSession;

/// Tool-exchange transcript cleanup.
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
}

#[cfg(test)]
mod tests {
    use crate::llm_session::ConversationSession;
    use closeclaw_common::ContentBlock;

    // ── remove_workflow_tool_exchange ────────────────────────────

    #[test]
    fn test_remove_tool_exchange_removes_verify_tool_call_and_result() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            std::path::PathBuf::from("/tmp"),
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
            std::path::PathBuf::from("/tmp"),
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
            std::path::PathBuf::from("/tmp"),
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
            std::path::PathBuf::from("/tmp"),
        );
        session.remove_workflow_tool_exchange(&["workflow_verify"]);
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_remove_tool_exchange_removes_empty_assistant_after_cleanup() {
        let mut session = ConversationSession::new(
            "sid".to_string(),
            "model".to_string(),
            std::path::PathBuf::from("/tmp"),
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
            std::path::PathBuf::from("/tmp"),
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
