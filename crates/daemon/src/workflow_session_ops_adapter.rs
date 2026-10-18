//! Production [`WorkflowSessionOps`] implementation for the daemon
//! composition root.
//!
//! [`SessionWorkflowOps`] bridges the workflow crate's control layer
//! ([`WorkflowEngineControl`](closeclaw_workflow::control::WorkflowEngineControl))
//! to the live `ConversationSession` instances held by the gateway
//! SessionManager: each operation looks the session up through the pub
//! gateway API, takes the session lock, and delegates to the session
//! crate's public workflow methods. The pause-kind classification compares
//! the stored run's paused reason against `DEFINITION_CHANGED_PAUSE_REASON`,
//! so the workflow crate never needs a session-crate dependency.

use std::sync::Arc;

use async_trait::async_trait;

use closeclaw_gateway::SessionManager;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::workflow_recovery::DEFINITION_CHANGED_PAUSE_REASON;
use closeclaw_workflow::control::{OwnerPauseKind, WorkflowSessionOps};

/// Adapter over [`SessionManager`] implementing the workflow
/// session-operations port.
pub struct SessionWorkflowOps {
    session_manager: Arc<SessionManager>,
}

/// Build the production session-ops adapter as `Arc<dyn WorkflowSessionOps>`.
pub fn session_workflow_ops(session_manager: Arc<SessionManager>) -> Arc<dyn WorkflowSessionOps> {
    Arc::new(SessionWorkflowOps { session_manager })
}

impl SessionWorkflowOps {
    /// Run a read-only closure under the session lock; `None` when the
    /// session does not exist.
    async fn with_session<R>(
        &self,
        session_id: &str,
        f: impl FnOnce(&ConversationSession) -> R,
    ) -> Option<R> {
        let cs = self
            .session_manager
            .get_conversation_session(session_id)
            .await?;
        let session = cs.read().await;
        Some(f(&session))
    }

    /// Run a mutating closure under the session lock; a no-op when the
    /// session does not exist.
    async fn mutate_session(&self, session_id: &str, f: impl FnOnce(&mut ConversationSession)) {
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            let mut session = cs.write().await;
            f(&mut session);
        }
    }
}

#[async_trait]
impl WorkflowSessionOps for SessionWorkflowOps {
    async fn owner_pause_kind(&self, session_id: &str) -> OwnerPauseKind {
        // A stored run blocks exactly when its paused reason is non-empty
        // (the engine clears the reason on resolve/terminate), so the
        // reason string alone classifies the pause kind.
        self.with_session(session_id, |session| {
            if session.workflow_run_value().is_none() {
                return OwnerPauseKind::NotBlocked;
            }
            match session.workflow_paused_reason().as_str() {
                "" => OwnerPauseKind::NotBlocked,
                DEFINITION_CHANGED_PAUSE_REASON => OwnerPauseKind::Terminable,
                _ => OwnerPauseKind::Resolvable,
            }
        })
        .await
        .unwrap_or(OwnerPauseKind::NotBlocked)
    }

    async fn read_workflow_run(&self, session_id: &str) -> Option<serde_json::Value> {
        self.with_session(session_id, |session| session.workflow_run_value())
            .await
            .flatten()
    }

    async fn remove_workflow_verify_messages(&self, session_id: &str) {
        self.mutate_session(session_id, |session| {
            session.remove_workflow_verify_messages();
        })
        .await;
    }

    async fn remove_workflow_messages(&self, session_id: &str) {
        self.mutate_session(session_id, |session| {
            session.remove_workflow_messages();
        })
        .await;
    }

    async fn remove_workflow_context_append(&self, session_id: &str) {
        self.mutate_session(session_id, |session| {
            session.remove_workflow_context_from_appends();
        })
        .await;
    }
}

#[cfg(test)]
#[path = "workflow_session_ops_adapter_tests.rs"]
mod tests;
