//! Independent confirmation flow for plan execution.
//!
//! Replaces the plan-exec metadata that was previously embedded in
//! [`ApprovalFlow`](closeclaw_permission::approval_flow::ApprovalFlow).
//! The confirmation flow is a standalone component that manages the
//! lifecycle of plan-execution confirmations — submit, confirm, cancel,
//! clear — without writing to the permission audit log.
//!
//! # Confirmation lifecycle
//!
//! ```text
//! Agent calls execute_plan
//!   → PlanExecConfirmFlow.submit(session_id, meta)
//!   → generates confirmation_id → stores in pending map (TTL)
//!   → on_notify sends confirmation card
//!   → returns { status: "confirm_pending", confirmation_id }
//!
//! Owner replies /confirm <id>
//!   → PlanExecConfirmFlow.confirm(id)
//!   → same-session: set_plan_state → set_session_mode(Auto) →
//!     mode-switch notification → inject additional_instruction
//!   → new-session: read plan file → create_child_session_fn →
//!     child session: plan_state + Auto + notification + instruction
//!
//! Owner replies /cancel <id>
//!   → PlanExecConfirmFlow.cancel(id) → session receives "已取消"
//!
//! Shutdown / clear
//!   → PlanExecConfirmFlow.clear() → all pending rejected
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_common::{PendingMessage, PlanPhase, SessionLookup, SessionMode};
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

// ── Public types ────────────────────────────────────────────────────────

/// Metadata for a plan execution confirmation request.
///
/// Migrated from `closeclaw_permission::approval_flow::PlanExecMetadata`.
#[derive(Debug, Clone)]
pub struct PlanExecMetadata {
    /// Path to the plan file to execute.
    pub plan_file_path: String,
    /// Optional step selection (0-based indices of steps to execute).
    pub step_selection: Option<Vec<usize>>,
    /// Whether to create a new child session for execution.
    pub new_session: bool,
    /// Optional additional instruction to inject as a user message
    /// when the plan enters Auto Mode.
    pub additional_instruction: Option<String>,
}

/// Callback type for creating a child session (new-session execution path).
///
/// Migrated from `closeclaw_permission::approval_flow::CreateChildSessionFn`.
///
/// # Arguments
/// * `parent_session_id` — ID of the session that requested plan execution.
/// * `plan_content` — Full content of the plan file to inject.
/// * `step_selection` — Optional step indices to execute.
///
/// # Returns
/// `Ok(new_session_id)` on success, `Err(message)` on failure.
pub type CreateChildSessionFn = Arc<
    dyn Fn(
            String,
            String,
            Option<Vec<usize>>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Notification payload sent to the owner when a confirmation is pending.
#[derive(Debug, Clone)]
pub struct PlanExecNotification {
    /// Unique confirmation identifier.
    pub confirmation_id: String,
    /// Path to the plan file being confirmed.
    pub plan_file_path: String,
    /// Whether a new session will be created.
    pub new_session: bool,
}

// ── Internal state ──────────────────────────────────────────────────────

/// Entry stored in the pending confirmation map.
#[derive(Debug, Clone)]
struct PlanExecConfirmEntry {
    /// Session that initiated the plan execution.
    session_id: String,
    /// Plan execution metadata.
    metadata: PlanExecMetadata,
}

/// Inner state protected by `Arc<TokioMutex<…>>`.
struct PendingState {
    /// Pending confirmations keyed by confirmation_id.
    pending: HashMap<String, PlanExecConfirmEntry>,
}

// ── PlanExecConfirmFlow ─────────────────────────────────────────────────

/// Independent confirmation flow for plan execution.
///
/// Manages the full lifecycle of plan-execution confirmations without
/// coupling to the permission approval chain.
pub struct PlanExecConfirmFlow {
    /// Shared pending state.
    state: TokioMutex<PendingState>,
    /// Session manager for pushing pending messages.
    session_manager: Arc<dyn SessionLookup>,
    /// Callback invoked to notify the owner about a pending confirmation.
    on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync>,
    /// Callback for creating child sessions (new-session path).
    create_child_session_fn: Option<CreateChildSessionFn>,
    /// Tokio runtime handle for spawning async tasks from sync closures.
    runtime_handle: tokio::runtime::Handle,
}

impl PlanExecConfirmFlow {
    /// Create a new `PlanExecConfirmFlow`.
    pub fn new(
        session_manager: Arc<dyn SessionLookup>,
        on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync>,
        runtime_handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            state: TokioMutex::new(PendingState {
                pending: HashMap::new(),
            }),
            session_manager,
            on_notify,
            create_child_session_fn: None,
            runtime_handle,
        }
    }

    /// Set the callback for creating child sessions.
    pub fn set_create_child_session_fn(&mut self, cb: CreateChildSessionFn) {
        self.create_child_session_fn = Some(cb);
    }

    /// Replace the owner notification callback.
    pub fn set_notify_callback(&mut self, cb: Arc<dyn Fn(PlanExecNotification) + Send + Sync>) {
        self.on_notify = cb;
    }
}

// ── Submit ──────────────────────────────────────────────────────────────

impl PlanExecConfirmFlow {
    /// Submit a plan execution confirmation request.
    pub async fn submit(&self, session_id: &str, metadata: PlanExecMetadata) -> String {
        let confirmation_id = Uuid::new_v4().to_string();

        {
            let mut state = self.state.lock().await;
            state.pending.insert(
                confirmation_id.clone(),
                PlanExecConfirmEntry {
                    session_id: session_id.to_string(),
                    metadata: metadata.clone(),
                },
            );
        }

        (self.on_notify)(PlanExecNotification {
            confirmation_id: confirmation_id.clone(),
            plan_file_path: metadata.plan_file_path,
            new_session: metadata.new_session,
        });

        confirmation_id
    }
}

// ── Confirm ─────────────────────────────────────────────────────────────

impl PlanExecConfirmFlow {
    /// Confirm a pending plan execution.
    ///
    /// Returns `true` if processed, `false` if unknown/already consumed.
    pub async fn confirm(&self, confirmation_id: &str) -> bool {
        let entry = {
            let mut state = self.state.lock().await;
            state.pending.remove(confirmation_id)
        };
        let entry = match entry {
            Some(e) => e,
            None => return false,
        };

        let sm = Arc::clone(&self.session_manager);
        let handle = self.runtime_handle.clone();
        let meta = entry.metadata;
        let session_id = entry.session_id;
        let create_child_fn = self.create_child_session_fn.clone();

        handle.spawn(async move {
            transition_plan_to_auto(&sm, &session_id, meta, &create_child_fn).await;
        });

        true
    }
}

// ── Cancel ──────────────────────────────────────────────────────────────

impl PlanExecConfirmFlow {
    /// Cancel a pending plan execution.
    ///
    /// Returns `true` if cancelled, `false` if unknown.
    pub async fn cancel(&self, confirmation_id: &str) -> bool {
        let entry = {
            let mut state = self.state.lock().await;
            state.pending.remove(confirmation_id)
        };
        let entry = match entry {
            Some(e) => e,
            None => return false,
        };

        let sm = Arc::clone(&self.session_manager);
        let handle = self.runtime_handle.clone();
        let session_id = entry.session_id;

        handle.spawn(async move {
            push_cancel_message(&sm, &session_id).await;
        });

        true
    }
}

// ── Clear ───────────────────────────────────────────────────────────────

impl PlanExecConfirmFlow {
    /// Clear all pending confirmations.
    pub async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.pending.clear();
    }
}

// ── Trait impl for Gateway ─────────────────────────────────────────────

#[async_trait::async_trait]
impl closeclaw_common::plan_confirm_handler::PlanConfirmationHandler for PlanExecConfirmFlow {
    async fn confirm(&self, confirmation_id: &str) -> bool {
        PlanExecConfirmFlow::confirm(self, confirmation_id).await
    }

    async fn cancel(&self, confirmation_id: &str) -> bool {
        PlanExecConfirmFlow::cancel(self, confirmation_id).await
    }
}

// ── Private helpers ─────────────────────────────────────────────────────

/// Push a cancellation message to the session.
async fn push_cancel_message(sm: &Arc<dyn SessionLookup>, session_id: &str) {
    let msg = PendingMessage::with_role(
        format!("confirm-cancel-{}", chrono::Utc::now().timestamp_millis()),
        "已取消执行 plan。".to_string(),
        "assistant".to_string(),
    );
    if let Err(e) = sm.push_pending_message(session_id, msg).await {
        tracing::warn!(
            session_id = %session_id,
            error = %e,
            "failed to push cancel message to session"
        );
    }
}

// ── Session path transitions ────────────────────────────────────────────

/// Transition the plan session to Auto Mode.
async fn transition_plan_to_auto(
    sm: &Arc<dyn SessionLookup>,
    session_id: &str,
    plan_meta: PlanExecMetadata,
    create_child_session_fn: &Option<CreateChildSessionFn>,
) {
    let mut plan_state = match sm.get_plan_state(session_id).await {
        Some(ps) => ps,
        None => return,
    };
    if plan_state.plan_file_path.is_empty() {
        return;
    }
    if plan_meta.new_session {
        handle_new_session_path(
            sm,
            session_id,
            &mut plan_state,
            &plan_meta,
            create_child_session_fn,
        )
        .await;
    } else {
        handle_same_session_path(sm, session_id, &plan_meta, &mut plan_state).await;
    }
}

/// Push the mode-switch notification to the session.
async fn push_mode_switch_notification(
    sm: &Arc<dyn SessionLookup>,
    session_id: &str,
    is_new_session: bool,
) {
    let label = if is_new_session {
        "✅ 已确认，进入 Auto Mode（新 session）"
    } else {
        "✅ 已确认，进入 Auto Mode"
    };
    let msg = PendingMessage::with_role(
        format!("confirm-mode-{}", chrono::Utc::now().timestamp_millis()),
        label.to_string(),
        "assistant".to_string(),
    );
    if let Err(e) = sm.push_pending_message(session_id, msg).await {
        tracing::warn!(
            session_id = %session_id,
            error = %e,
            "failed to push mode switch notification"
        );
    }
}

/// Inject an additional instruction as a user pending message.
async fn inject_additional_instruction(
    sm: &Arc<dyn SessionLookup>,
    session_id: &str,
    plan_meta: &PlanExecMetadata,
) {
    let instruction = match &plan_meta.additional_instruction {
        Some(i) if !i.trim().is_empty() => i.clone(),
        _ => return,
    };
    let msg = PendingMessage::with_role(
        format!(
            "additional-instruction-{}",
            chrono::Utc::now().timestamp_millis()
        ),
        instruction,
        "user".to_string(),
    );
    if let Err(e) = sm.push_pending_message(session_id, msg).await {
        tracing::warn!(
            session_id = %session_id,
            error = %e,
            "failed to inject additional instruction"
        );
    }
}

// ── Same-session path ───────────────────────────────────────────────────

async fn handle_same_session_path(
    sm: &Arc<dyn SessionLookup>,
    session_id: &str,
    plan_meta: &PlanExecMetadata,
    plan_state: &mut closeclaw_common::PlanState,
) {
    plan_state.phase = PlanPhase::FinalPlan;
    sm.set_plan_state(session_id, plan_state.clone()).await;
    sm.set_session_mode(session_id, SessionMode::Auto).await;
    push_mode_switch_notification(sm, session_id, false).await;
    inject_additional_instruction(sm, session_id, plan_meta).await;
}

// ── New-session path ────────────────────────────────────────────────────

async fn handle_new_session_path(
    sm: &Arc<dyn SessionLookup>,
    session_id: &str,
    plan_state: &mut closeclaw_common::PlanState,
    plan_meta: &PlanExecMetadata,
    create_child_session_fn: &Option<CreateChildSessionFn>,
) {
    let plan_file_path = plan_state.plan_file_path.clone();
    let plan_content = match read_plan_file_for_injection(&plan_file_path).await {
        Some(c) => c,
        None => return,
    };

    let new_session_id = match create_child_session_fn {
        Some(ref create_fn) => {
            match invoke_create_child_session(
                create_fn,
                session_id,
                plan_content,
                &plan_meta.step_selection,
            )
            .await
            {
                Ok(id) => id,
                Err(()) => return,
            }
        }
        None => {
            tracing::info!(
                parent_session = %session_id,
                "no create_child_session_fn, fallback to same-session"
            );
            plan_state.phase = PlanPhase::FinalPlan;
            sm.set_plan_state(session_id, plan_state.clone()).await;
            return;
        }
    };

    let child_plan_state = setup_child_plan_state(&plan_file_path);
    sm.set_plan_state(&new_session_id, child_plan_state).await;
    sm.set_session_mode(&new_session_id, SessionMode::Auto)
        .await;
    push_mode_switch_notification(sm, &new_session_id, true).await;
    inject_additional_instruction(sm, &new_session_id, plan_meta).await;
}

/// Read plan file content for injection into a child session.
async fn read_plan_file_for_injection(path: &str) -> Option<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Some(content),
        Err(e) => {
            tracing::warn!(
                plan_file = %path,
                error = %e,
                "failed to read plan file for new session injection"
            );
            None
        }
    }
}

/// Create a [`PlanState`](closeclaw_common::PlanState) for the child session.
fn setup_child_plan_state(path: &str) -> closeclaw_common::PlanState {
    let mut state = closeclaw_common::PlanState::new();
    state.plan_file_path = path.to_string();
    state.phase = PlanPhase::FinalPlan;
    state
}

/// Create a child session via the injected callback.
async fn invoke_create_child_session(
    create_fn: &CreateChildSessionFn,
    parent_session_id: &str,
    plan_content: String,
    step_selection: &Option<Vec<usize>>,
) -> Result<String, ()> {
    create_fn(
        parent_session_id.to_string(),
        plan_content,
        step_selection.clone(),
    )
    .await
    .map_err(|e| {
        tracing::warn!(
            parent_session = %parent_session_id,
            error = %e,
            "failed to create child session"
        );
    })
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::PlanState;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    /// Shared observable state for the mock.
    struct MockState {
        pending_messages: Vec<(String, PendingMessage)>,
        plan_states: HashMap<String, PlanState>,
        modes: HashMap<String, SessionMode>,
    }

    /// Minimal mock implementing [`SessionLookup`] for unit tests.
    struct MockSessionLookup {
        inner: StdMutex<MockState>,
    }

    impl MockSessionLookup {
        fn new() -> Self {
            Self {
                inner: StdMutex::new(MockState {
                    pending_messages: Vec::new(),
                    plan_states: HashMap::new(),
                    modes: HashMap::new(),
                }),
            }
        }

        fn state(&self) -> std::sync::MutexGuard<'_, MockState> {
            self.inner.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl SessionLookup for MockSessionLookup {
        async fn get_parent_of(&self, _child_id: &str) -> Option<String> {
            None
        }

        async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
            None
        }

        async fn push_pending_message(
            &self,
            session_id: &str,
            msg: PendingMessage,
        ) -> Result<(), String> {
            self.state()
                .pending_messages
                .push((session_id.to_string(), msg));
            Ok(())
        }

        async fn get_plan_state(&self, session_id: &str) -> Option<PlanState> {
            self.state().plan_states.get(session_id).cloned()
        }

        async fn set_plan_state(&self, session_id: &str, plan_state: PlanState) {
            self.state()
                .plan_states
                .insert(session_id.to_string(), plan_state);
        }

        async fn set_session_mode(&self, session_id: &str, mode: SessionMode) {
            self.state().modes.insert(session_id.to_string(), mode);
        }
    }

    /// Build a concrete mock + Arc<dyn> pair so tests can inspect state.
    fn make_mock() -> (Arc<MockSessionLookup>, Arc<dyn SessionLookup>) {
        let mock = Arc::new(MockSessionLookup::new());
        let sm: Arc<dyn SessionLookup> = mock.clone();
        (mock, sm)
    }

    fn make_test_meta(plan_file_path: &str) -> PlanExecMetadata {
        PlanExecMetadata {
            plan_file_path: plan_file_path.to_string(),
            step_selection: None,
            new_session: false,
            additional_instruction: None,
        }
    }

    fn insert_plan_state(mock: &MockSessionLookup, session_id: &str, path: &str) {
        let mut ps = PlanState::new();
        ps.plan_file_path = path.to_string();
        mock.state().plan_states.insert(session_id.to_string(), ps);
    }

    #[tokio::test]
    async fn submit_generates_id_and_stores_entry() {
        let (_mock, sm) = make_mock();
        let notify_count = Arc::new(AtomicUsize::new(0));
        let nc = Arc::clone(&notify_count);
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(move |_| {
            nc.fetch_add(1, Ordering::SeqCst);
        });

        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());
        let meta = make_test_meta("/tmp/test_plan.md");
        let id = flow.submit("session-1", meta).await;

        assert!(!id.is_empty());
        assert_eq!(notify_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn confirm_unknown_id_returns_false() {
        let (_mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        assert!(!flow.confirm("nonexistent").await);
    }

    #[tokio::test]
    async fn cancel_unknown_id_returns_false() {
        let (_mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        assert!(!flow.cancel("nonexistent").await);
    }

    #[tokio::test]
    async fn clear_removes_all_pending() {
        let (_mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let mut flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        let create_fn: CreateChildSessionFn =
            Arc::new(|_, _, _| Box::pin(async { Ok("child-1".to_string()) }));
        flow.set_create_child_session_fn(create_fn);

        let meta = make_test_meta("/tmp/plan.md");
        let id = flow.submit("session-1", meta).await;

        flow.clear().await;

        assert!(!flow.confirm(&id).await);
    }

    #[tokio::test]
    async fn confirm_same_session_transitions_to_auto() {
        let (mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        insert_plan_state(&mock, "session-1", "/tmp/plan.md");

        let meta = PlanExecMetadata {
            plan_file_path: "/tmp/plan.md".to_string(),
            step_selection: None,
            new_session: false,
            additional_instruction: None,
        };
        let id = flow.submit("session-1", meta).await;

        assert!(flow.confirm(&id).await);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let state = mock.state();
        assert_eq!(state.modes.get("session-1"), Some(&SessionMode::Auto));
    }

    #[tokio::test]
    async fn confirm_same_session_with_additional_instruction() {
        let (mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        insert_plan_state(&mock, "session-1", "/tmp/plan.md");

        let meta = PlanExecMetadata {
            plan_file_path: "/tmp/plan.md".to_string(),
            step_selection: None,
            new_session: false,
            additional_instruction: Some("请先阅读 CONTRIBUTING.md".to_string()),
        };
        let id = flow.submit("session-1", meta).await;

        assert!(flow.confirm(&id).await);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let state = mock.state();
        let session_msgs: Vec<&PendingMessage> = state
            .pending_messages
            .iter()
            .filter(|(sid, _)| sid == "session-1")
            .map(|(_, m)| m)
            .collect();

        assert_eq!(session_msgs.len(), 2);
        assert!(session_msgs[0].content.contains("Auto Mode"));
        assert_eq!(session_msgs[1].content, "请先阅读 CONTRIBUTING.md");
        assert_eq!(session_msgs[1].role.as_deref(), Some("user"));
    }

    #[tokio::test]
    async fn confirm_new_session_fallback_to_same_session() {
        let (mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        // Create a temp plan file so read_plan_file_for_injection succeeds
        let tmp_dir = tempfile::tempdir().unwrap();
        let plan_path = tmp_dir.path().join("plan.md");
        std::fs::write(&plan_path, "# Plan\nStep 1").unwrap();
        let plan_path_str = plan_path.to_string_lossy().to_string();

        insert_plan_state(&mock, "session-1", &plan_path_str);

        let meta = PlanExecMetadata {
            plan_file_path: plan_path_str,
            step_selection: None,
            new_session: true,
            additional_instruction: None,
        };
        let id = flow.submit("session-1", meta).await;

        assert!(flow.confirm(&id).await);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let state = mock.state();
        let ps = state.plan_states.get("session-1").unwrap();
        assert_eq!(ps.phase, PlanPhase::FinalPlan);
    }

    #[tokio::test]
    async fn cancel_removes_entry_and_pushes_message() {
        let (mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        let meta = make_test_meta("/tmp/plan.md");
        let id = flow.submit("session-1", meta).await;

        assert!(flow.cancel(&id).await);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let state = mock.state();
        let session_msgs: Vec<&str> = state
            .pending_messages
            .iter()
            .filter(|(sid, _)| sid == "session-1")
            .map(|(_, m)| m.content.as_str())
            .collect();

        assert!(session_msgs.iter().any(|m| m.contains("已取消执行 plan")));
        assert!(!flow.confirm(&id).await);
    }

    #[tokio::test]
    async fn confirm_idempotent_second_confirm_returns_false() {
        let (mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        insert_plan_state(&mock, "session-1", "/tmp/plan.md");

        let meta = make_test_meta("/tmp/plan.md");
        let id = flow.submit("session-1", meta).await;

        assert!(flow.confirm(&id).await);
        assert!(!flow.confirm(&id).await);
    }

    #[tokio::test]
    async fn additional_instruction_whitespace_ignored() {
        let (mock, sm) = make_mock();
        let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
        let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

        insert_plan_state(&mock, "session-1", "/tmp/plan.md");

        let meta = PlanExecMetadata {
            plan_file_path: "/tmp/plan.md".to_string(),
            step_selection: None,
            new_session: false,
            additional_instruction: Some("   ".to_string()),
        };
        let id = flow.submit("session-1", meta).await;

        assert!(flow.confirm(&id).await);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let state = mock.state();
        let session_msgs: Vec<&PendingMessage> = state
            .pending_messages
            .iter()
            .filter(|(sid, _)| sid == "session-1")
            .map(|(_, m)| m)
            .collect();

        assert_eq!(session_msgs.len(), 1);
        assert!(session_msgs[0].content.contains("Auto Mode"));
    }
}
