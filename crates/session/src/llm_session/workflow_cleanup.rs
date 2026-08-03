//! Workflow exit cleanup for [`ConversationSession`].
//!
//! Provides [`ConversationSession::cleanup_workflow_exit`] — the single
//! entry-point for workflow teardown when a workflow completes or is
//! terminated by the owner.

use super::ConversationSession;

/// Workflow exit cleanup methods for [`ConversationSession`].
impl ConversationSession {
    /// Perform the full workflow exit cleanup on this session.
    ///
    /// Coordinates all four cleanup steps:
    ///
    /// 1. Remove workflow context markers from `system_appends`
    ///    (delegates to [`crate::workflow_recovery::cleanup_workflow_exit`]).
    /// 2. Remove workflow control messages (role == `"workflow"`) from the
    ///    in-memory transcript.
    /// 3. Clear `workflow_run` and `workflow_handler` to `None`.
    /// 4. Persist the cleaned checkpoint if `checkpoint_storage` is set.
    ///
    /// This is the single entry-point for workflow teardown, called by:
    /// - The workflow engine when phase transitions to `Complete`.
    /// - The owner terminate flow.
    pub async fn cleanup_workflow_exit(&mut self) {
        use crate::workflow_recovery::{cleanup_workflow_exit as cp_cleanup, WorkflowExitReport};

        // 1 & 3: Checkpoint-level cleanup (system_appends + workflow_run).
        // Build a temporary checkpoint to apply the cleanup, then merge
        // the results back into the session state.
        let mut cp = self.build_cleanup_checkpoint();
        let _report: WorkflowExitReport = cp_cleanup(&mut cp);
        self.apply_cleanup_checkpoint(&cp);

        // 2: Remove workflow control messages from in-memory transcript.
        self.remove_workflow_messages();

        // 3: Clear runtime workflow state.
        self.clear_workflow_run();

        // 4: Persist the cleaned checkpoint if storage is available.
        if let Some(ref storage) = self.checkpoint_storage {
            let mut persist_cp = cp;
            persist_cp.session_id = self.session_id.clone();
            persist_cp.touch();
            if let Err(e) = storage.save_checkpoint(&persist_cp).await {
                tracing::warn!(
                    session_id = %self.session_id,
                    "failed to persist checkpoint after workflow exit cleanup: {}",
                    e,
                );
            } else {
                tracing::info!(
                    session_id = %self.session_id,
                    "checkpoint persisted after workflow exit cleanup"
                );
            }
        }
    }

    /// Build a temporary [`SessionCheckpoint`] from session state for cleanup.
    fn build_cleanup_checkpoint(&self) -> crate::persistence::SessionCheckpoint {
        use crate::persistence::SessionCheckpoint;
        let mut cp = SessionCheckpoint::new(self.session_id.clone());
        cp.system_appends = self.user_system_appends().to_vec();
        cp.workflow_run = self.workflow_run().cloned();
        cp
    }

    /// Apply cleanup results from a checkpoint back into session state.
    fn apply_cleanup_checkpoint(&mut self, cp: &crate::persistence::SessionCheckpoint) {
        self.restore_system_appends(cp.system_appends.clone());
        self.set_workflow_run(cp.workflow_run.clone());
    }
}

#[cfg(test)]
mod tests {
    use crate::llm_session::ConversationSession;
    use closeclaw_common::ContentBlock;
    use closeclaw_workflow::context_append::build_workflow_context_append;
    use closeclaw_workflow::definition::{Step, Workflow};
    use closeclaw_workflow::run::{Phase, StepHistoryEntry, WorkflowRun};
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
                name: "Step Zero".to_string(),
                allow_blocked: None,
                goal: "Do the first thing".to_string(),
                verify: vec![],
                jump: vec![],
                transitions: vec![],
            }],
        }
    }

    fn make_test_run() -> WorkflowRun {
        WorkflowRun {
            workflow_id: "test-wf".to_string(),
            definition_name: "Test Workflow".to_string(),
            definition_version: "0.1".to_string(),
            current_step: 0,
            phase: Phase::Executing,
            step_history: vec![StepHistoryEntry {
                step_id: 0,
                step_name: "Step Zero".to_string(),
                completed_at: "2026-01-01T00:00:00Z".to_string(),
            }],
            step_data: Default::default(),
            pending_verify: 0,
        }
    }

    #[tokio::test]
    async fn test_cleanup_removes_workflow_messages() {
        let mut session = ConversationSession::new(
            "test-sid".to_string(),
            "test-model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.set_workflow_run(Some(make_test_run()));
        session.inject_workflow_message("goal: do something");
        session.push_message("user", vec![ContentBlock::Text("hello".to_string())]);
        session.inject_workflow_message("recovered: resuming");

        assert_eq!(session.messages.len(), 3);
        assert!(session.messages.iter().any(|m| m.role == "workflow"));

        session.cleanup_workflow_exit().await;

        assert!(session.messages.iter().all(|m| m.role != "workflow"));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, "user");
    }

    #[tokio::test]
    async fn test_cleanup_removes_workflow_context_from_appends() {
        let mut session = ConversationSession::new(
            "test-sid".to_string(),
            "test-model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.set_workflow_run(Some(make_test_run()));
        session.add_system_append(build_workflow_context_append(&make_test_workflow()));
        session.add_system_append("user-append".to_string());

        session.cleanup_workflow_exit().await;

        let appends = session.user_system_appends();
        assert!(appends.iter().all(|s| !s.starts_with("--- WORKFLOW ---")));
        assert!(appends.contains(&"user-append".to_string()));
    }

    #[tokio::test]
    async fn test_cleanup_clears_workflow_run() {
        let mut session = ConversationSession::new(
            "test-sid".to_string(),
            "test-model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.set_workflow_run(Some(make_test_run()));

        session.cleanup_workflow_exit().await;

        assert!(session.workflow_run().is_none());
        assert!(session.workflow_handler().is_none());
    }

    #[tokio::test]
    async fn test_cleanup_without_workflow_run() {
        let mut session = ConversationSession::new(
            "test-sid".to_string(),
            "test-model".to_string(),
            PathBuf::from("/tmp"),
        );
        // No workflow_run set.

        session.cleanup_workflow_exit().await;

        assert!(session.workflow_run().is_none());
    }

    #[tokio::test]
    async fn test_cleanup_preserves_user_messages() {
        let mut session = ConversationSession::new(
            "test-sid".to_string(),
            "test-model".to_string(),
            PathBuf::from("/tmp"),
        );
        session.set_workflow_run(Some(make_test_run()));
        session.push_message("user", vec![ContentBlock::Text("question".to_string())]);
        session.inject_workflow_message("goal: step 0");
        session.push_message("assistant", vec![ContentBlock::Text("answer".to_string())]);

        session.cleanup_workflow_exit().await;

        // Only user and assistant messages remain.
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, "user");
        assert_eq!(session.messages[1].role, "assistant");
    }
}
