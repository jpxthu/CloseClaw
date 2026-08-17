//! Plan execution approval flow logic.
//!
//! Extracted from `approval_flow.rs` to keep that file under the
//! 1000-line limit.

use crate::approval_flow::{ApprovalFlow, CreateChildSessionFn, PlanExecMetadata};
use closeclaw_common::{PendingMessage, PlanPhase, SessionMode};
use std::sync::Arc;

impl ApprovalFlow {
    /// Handle execute plan approval: push result and transition
    /// session to Auto Mode.
    pub(crate) async fn handle_plan_exec_approval(
        &mut self,
        request_id: &str,
        pending_info: &Option<super::PendingInfo>,
        result: bool,
    ) {
        if !result {
            return;
        }
        let session_id = match pending_info {
            Some((sid, _, _, _, _, _)) => sid.clone(),
            None => return,
        };
        let sm = Arc::clone(&self.session_manager);
        let handle = self.runtime_handle.clone();
        let rid = request_id.to_string();
        let plan_meta = self.plan_exec_metadata.remove(&rid);
        let create_child_fn = self.create_child_session_fn.clone();

        handle.spawn(async move {
            Self::push_approval_result(&sm, &session_id, &rid).await;
            Self::transition_plan_to_auto(&sm, &session_id, plan_meta, &create_child_fn).await;
        });
    }

    /// Push the approval result message to the session.
    async fn push_approval_result(
        sm: &Arc<dyn closeclaw_common::SessionLookup>,
        session_id: &str,
        rid: &str,
    ) {
        let content = format!("[审批 {}] 操作已批准", rid);
        let msg = PendingMessage::with_role(
            format!("approval-{}", chrono::Utc::now().timestamp_millis()),
            content,
            "assistant".to_string(),
        );
        if let Err(e) = sm.push_pending_message(session_id, msg).await {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "failed to push approval result to session"
            );
        }
    }

    /// Transition the plan session to Auto Mode (same-session or
    /// new-session).
    async fn transition_plan_to_auto(
        sm: &Arc<dyn closeclaw_common::SessionLookup>,
        session_id: &str,
        plan_meta: Option<PlanExecMetadata>,
        create_child_session_fn: &Option<CreateChildSessionFn>,
    ) {
        let mut plan_state = match sm.get_plan_state(session_id).await {
            Some(ps) => ps,
            None => return,
        };
        if plan_state.plan_file_path.is_empty() {
            return;
        }
        let is_new_session = plan_meta.as_ref().map(|m| m.new_session).unwrap_or(false);
        if is_new_session {
            Self::handle_new_session_path(
                sm,
                session_id,
                &mut plan_state,
                &plan_meta,
                create_child_session_fn,
            )
            .await;
        } else {
            Self::handle_same_session_path(sm, session_id, &mut plan_state).await;
        }
    }
}

// ── Plan session path helpers ─────────────────────────────────────

impl ApprovalFlow {
    /// Read plan file content for injection into a child session.
    async fn read_plan_file_for_injection(path: &str) -> Option<String> {
        match tokio::fs::read_to_string(path).await {
            Ok(content) => Some(content),
            Err(e) => {
                tracing::warn!(
                    plan_file = %path,
                    error = %e,
                    "failed to read plan file for \
                     new session injection"
                );
                None
            }
        }
    }

    /// Create a [`PlanState`] configured for the child session.
    fn setup_child_plan_state(path: &str) -> closeclaw_common::PlanState {
        let mut state = closeclaw_common::PlanState::new();
        state.plan_file_path = path.to_string();
        state.phase = PlanPhase::FinalPlan;
        state
    }

    /// Fallback when no `create_child_session_fn` is configured.
    /// Updates plan state on the parent session (same-session
    /// behavior).
    async fn handle_new_session_fallback(
        sm: &Arc<dyn closeclaw_common::SessionLookup>,
        session_id: &str,
        plan_state: &mut closeclaw_common::PlanState,
    ) {
        tracing::info!(
            parent_session = %session_id,
            "no create_child_session_fn, fallback \
             to same-session"
        );
        plan_state.phase = PlanPhase::FinalPlan;
        sm.set_plan_state(session_id, plan_state.clone()).await;
    }

    /// Push the mode-switch notification to the new child session.
    async fn notify_new_session_mode_switch(
        sm: &Arc<dyn closeclaw_common::SessionLookup>,
        new_session_id: &str,
    ) {
        let mode_msg = PendingMessage::with_role(
            format!("approval-mode-{}", chrono::Utc::now().timestamp_millis()),
            "✅ Plan approved, entering Auto Mode \
             (new session)"
                .to_string(),
            "assistant".to_string(),
        );
        if let Err(e) = sm.push_pending_message(new_session_id, mode_msg).await {
            tracing::warn!(
                session_id = %new_session_id,
                error = %e,
                "failed to push mode switch notification"
            );
        }
    }
}

// ── Child session creation callback ──────────────────────────────

impl ApprovalFlow {
    /// Create a child session via the injected callback.
    ///
    /// Returns `Ok(new_session_id)` on success, `Err` with logging
    /// on failure.
    async fn invoke_create_child_session(
        create_fn: &CreateChildSessionFn,
        parent_session_id: &str,
        plan_content: String,
        plan_meta: &Option<PlanExecMetadata>,
    ) -> Result<String, ()> {
        let step_selection = plan_meta.as_ref().and_then(|m| m.step_selection.clone());
        create_fn(parent_session_id.to_string(), plan_content, step_selection)
            .await
            .map_err(|e| {
                tracing::warn!(
                    parent_session = %parent_session_id,
                    error = %e,
                    "failed to create child session"
                );
            })
    }
}

// ── New session path ─────────────────────────────────────────────

impl ApprovalFlow {
    /// Handle new-session execution path: create a child session
    /// with plan content injected as initial context, then enter
    /// Auto Mode.
    ///
    /// Falls back to same-session behavior when no callback is
    /// configured.
    async fn handle_new_session_path(
        sm: &Arc<dyn closeclaw_common::SessionLookup>,
        session_id: &str,
        plan_state: &mut closeclaw_common::PlanState,
        plan_meta: &Option<PlanExecMetadata>,
        create_child_session_fn: &Option<CreateChildSessionFn>,
    ) {
        let plan_file_path = plan_state.plan_file_path.clone();
        let plan_content = match Self::read_plan_file_for_injection(&plan_file_path).await {
            Some(c) => c,
            None => return,
        };

        let new_session_id = match create_child_session_fn {
            Some(ref create_fn) => {
                let r = Self::invoke_create_child_session(
                    create_fn,
                    session_id,
                    plan_content,
                    plan_meta,
                )
                .await;
                match r {
                    Ok(id) => id,
                    Err(()) => return,
                }
            }
            None => {
                Self::handle_new_session_fallback(sm, session_id, plan_state).await;
                return;
            }
        };

        let child_plan_state = Self::setup_child_plan_state(&plan_file_path);
        sm.set_plan_state(&new_session_id, child_plan_state).await;
        sm.set_session_mode(&new_session_id, SessionMode::Auto)
            .await;
        Self::notify_new_session_mode_switch(sm, &new_session_id).await;
    }
}

// ── Session path transitions ─────────────────────────────────────

impl ApprovalFlow {
    /// Handle same-session execution path: transition to Auto Mode.
    async fn handle_same_session_path(
        sm: &Arc<dyn closeclaw_common::SessionLookup>,
        session_id: &str,
        plan_state: &mut closeclaw_common::PlanState,
    ) {
        plan_state.phase = PlanPhase::FinalPlan;
        sm.set_plan_state(session_id, plan_state.clone()).await;
        sm.set_session_mode(session_id, SessionMode::Auto).await;
        let mode_msg = PendingMessage::with_role(
            format!("approval-mode-{}", chrono::Utc::now().timestamp_millis()),
            "✅ Plan approved, entering Auto Mode".to_string(),
            "assistant".to_string(),
        );
        if let Err(e) = sm.push_pending_message(session_id, mode_msg).await {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "failed to push mode switch notification"
            );
        }
    }
}
