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
                DEFINITION_CHANGED_PAUSE_REASON => OwnerPauseKind::Terminological,
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
mod tests {
    use super::*;

    use closeclaw_gateway::GatewayConfig;
    use closeclaw_workflow::run::{GoalHint, PendingVerify, Phase, WorkflowRun};

    fn make_session_manager() -> Arc<SessionManager> {
        Arc::new(SessionManager::new(
            &GatewayConfig::default(),
            None,
            None,
            closeclaw_common::ReasoningLevel::default(),
        ))
    }

    async fn insert_session(mgr: &Arc<SessionManager>, session_id: &str) {
        mgr.conversation_sessions.write().await.insert(
            session_id.to_string(),
            Arc::new(tokio::sync::RwLock::new(ConversationSession::new(
                session_id.to_string(),
                "stub-model".to_string(),
                std::env::temp_dir().join("workflow-ops-tests"),
            ))),
        );
    }

    /// Serialized run snapshot in the given phase with the given pause reason.
    fn run_value(phase: Phase, paused_reason: &str) -> serde_json::Value {
        let run = WorkflowRun {
            workflow_id: "run-1".into(),
            definition_name: "test-workflow".into(),
            definition_version: "0.1".into(),
            current_step: 0,
            phase,
            current_step_entered_at: String::new(),
            step_history: Vec::new(),
            step_data: serde_yaml::Value::Null,
            pending_goal_hint: GoalHint::Normal,
            pending_verify: PendingVerify::default(),
            paused_reason: paused_reason.to_string(),
        };
        serde_json::to_value(&run).unwrap()
    }

    async fn set_run(mgr: &Arc<SessionManager>, session_id: &str, value: serde_json::Value) {
        let cs = mgr
            .get_conversation_session(session_id)
            .await
            .expect("session inserted");
        let mut session = cs.write().await;
        session.set_workflow_port(crate::workflow_port_adapter::engine_workflow_port());
        session.set_workflow_run_value(Some(value));
    }

    // ── owner_pause_kind（暂停类型判定） ────────────────────────────────

    #[tokio::test]
    async fn missing_session_classifies_as_not_blocked() {
        let mgr = make_session_manager();
        let ops = session_workflow_ops(mgr);
        assert_eq!(
            ops.owner_pause_kind("no-such-session").await,
            OwnerPauseKind::NotBlocked
        );
    }

    #[tokio::test]
    async fn no_run_classifies_as_not_blocked() {
        let mgr = make_session_manager();
        insert_session(&mgr, "s1").await;
        let ops = session_workflow_ops(mgr);
        assert_eq!(ops.owner_pause_kind("s1").await, OwnerPauseKind::NotBlocked);
    }

    #[tokio::test]
    async fn running_phase_classifies_as_not_blocked() {
        let mgr = make_session_manager();
        insert_session(&mgr, "s1").await;
        set_run(&mgr, "s1", run_value(Phase::Executing, "")).await;
        let ops = session_workflow_ops(mgr);
        assert_eq!(ops.owner_pause_kind("s1").await, OwnerPauseKind::NotBlocked);
    }

    #[tokio::test]
    async fn definition_change_pause_is_terminological() {
        let mgr = make_session_manager();
        insert_session(&mgr, "s1").await;
        set_run(
            &mgr,
            "s1",
            run_value(Phase::Blocked, DEFINITION_CHANGED_PAUSE_REASON),
        )
        .await;
        let ops = session_workflow_ops(mgr);
        assert_eq!(
            ops.owner_pause_kind("s1").await,
            OwnerPauseKind::Terminological
        );
    }

    #[tokio::test]
    async fn other_pause_reason_is_resolvable() {
        let mgr = make_session_manager();
        insert_session(&mgr, "s1").await;
        set_run(&mgr, "s1", run_value(Phase::Blocked, "验收重试次数耗尽")).await;
        let ops = session_workflow_ops(mgr);
        assert_eq!(ops.owner_pause_kind("s1").await, OwnerPauseKind::Resolvable);
    }

    // ── read_workflow_run（类型擦除 run 读取） ──────────────────────────

    #[tokio::test]
    async fn read_workflow_run_returns_stored_value() {
        let mgr = make_session_manager();
        insert_session(&mgr, "s1").await;
        let value = run_value(Phase::Verifying, "");
        set_run(&mgr, "s1", value.clone()).await;
        let ops = session_workflow_ops(mgr);
        assert_eq!(ops.read_workflow_run("s1").await, Some(value));
    }

    #[tokio::test]
    async fn read_workflow_run_missing_session_is_none() {
        let mgr = make_session_manager();
        let ops = session_workflow_ops(mgr);
        assert_eq!(ops.read_workflow_run("no-such-session").await, None);
    }

    // ── context append 清理（pub getter 可观测） ────────────────────────

    #[tokio::test]
    async fn remove_workflow_context_append_clears_only_workflow_items() {
        let mgr = make_session_manager();
        insert_session(&mgr, "s1").await;
        {
            let cs = mgr.get_conversation_session("s1").await.unwrap();
            let mut session = cs.write().await;
            session.add_system_injection_append(
                "--- WORKFLOW ---\nctx\n--- WORKFLOW END ---".to_string(),
            );
            session.add_system_injection_append("unrelated append".to_string());
        }
        let ops = session_workflow_ops(mgr.clone());
        ops.remove_workflow_context_append("s1").await;
        let cs = mgr.get_conversation_session("s1").await.unwrap();
        let session = cs.read().await;
        assert_eq!(session.system_injection_appends(), &["unrelated append"]);
    }
}
