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
