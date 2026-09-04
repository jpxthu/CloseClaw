use super::*;
use closeclaw_common::PlanState;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

/// Shared observable state for the mock.
struct MockState {
    pending_messages: Vec<(String, PendingMessage)>,
    plan_states: HashMap<String, PlanState>,
    modes: HashMap<String, SessionMode>,
    pending_modes: HashMap<String, SessionMode>,
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
                pending_modes: HashMap::new(),
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

    async fn set_pending_session_mode(&self, session_id: &str, mode: SessionMode) {
        self.state()
            .pending_modes
            .insert(session_id.to_string(), mode);
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
    assert_eq!(
        state.pending_modes.get("session-1"),
        Some(&SessionMode::Auto)
    );
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

// ── Missing dimension: submit stores correct metadata ─────────────

#[tokio::test]
async fn submit_stores_correct_metadata_in_pending_map() {
    let (_mock, sm) = make_mock();
    let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
    let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

    let meta = PlanExecMetadata {
        plan_file_path: "/plans/my-plan.md".to_string(),
        step_selection: Some(vec![0, 2]),
        new_session: true,
        additional_instruction: Some("do X first".to_string()),
    };
    let id = flow.submit("sess-1", meta).await;

    let stored = flow
        .get_pending_metadata(&id)
        .await
        .expect("entry should exist");
    assert_eq!(stored.plan_file_path, "/plans/my-plan.md");
    assert_eq!(stored.step_selection, Some(vec![0, 2]));
    assert!(stored.new_session);
    assert_eq!(stored.additional_instruction.as_deref(), Some("do X first"));
}

#[tokio::test]
async fn submit_notifies_with_correct_payload() {
    let (_mock, sm) = make_mock();
    let captured: Arc<std::sync::Mutex<Vec<PlanExecNotification>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let cc = Arc::clone(&captured);
    let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> =
        Arc::new(move |n| cc.lock().unwrap().push(n));

    let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());
    let meta = PlanExecMetadata {
        plan_file_path: "/plans/demo.md".to_string(),
        step_selection: None,
        new_session: false,
        additional_instruction: None,
    };
    let id = flow.submit("s", meta).await;

    let notifications = captured.lock().unwrap();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].confirmation_id, id);
    assert_eq!(notifications[0].plan_file_path, "/plans/demo.md");
    assert!(!notifications[0].new_session);
}

// ── Missing dimension: new-session path with callback ─────────────

#[tokio::test]
async fn confirm_new_session_with_callback_creates_child() {
    let (mock, sm) = make_mock();
    let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
    let mut flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

    // Create a temp plan file so read_plan_file_for_injection succeeds.
    let tmp_dir = tempfile::tempdir().unwrap();
    let plan_path = tmp_dir.path().join("plan.md");
    std::fs::write(&plan_path, "# Plan\nStep 1").unwrap();
    let plan_path_str = plan_path.to_string_lossy().to_string();

    insert_plan_state(&mock, "parent-sess", &plan_path_str);

    // Track callback invocation.
    let callback_invoked = Arc::new(AtomicBool::new(false));
    let ci = Arc::clone(&callback_invoked);
    let captured_steps: Arc<std::sync::Mutex<Option<Vec<usize>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let cs = Arc::clone(&captured_steps);
    let create_fn: CreateChildSessionFn = Arc::new(move |parent, _plan, steps| {
        ci.store(true, Ordering::SeqCst);
        *cs.lock().unwrap() = steps;
        Box::pin(async move { Ok(format!("child-of-{parent}")) })
    });
    flow.set_create_child_session_fn(create_fn);

    let meta = PlanExecMetadata {
        plan_file_path: plan_path_str,
        step_selection: Some(vec![1, 3]),
        new_session: true,
        additional_instruction: None,
    };
    let id = flow.submit("parent-sess", meta).await;
    assert!(flow.confirm(&id).await);

    // Wait for the spawned task to complete.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert!(
        callback_invoked.load(Ordering::SeqCst),
        "create_child_session_fn should have been called"
    );
    assert_eq!(
        *captured_steps.lock().unwrap(),
        Some(vec![1, 3]),
        "step_selection should be forwarded to callback"
    );

    // Child session should have been set up.
    let state = mock.state();
    let child_id = "child-of-parent-sess";
    assert_eq!(state.modes.get(child_id), Some(&SessionMode::Auto));
    let child_ps = state.plan_states.get(child_id).expect("child plan state");
    assert_eq!(child_ps.phase, PlanPhase::FinalPlan);
}

// ── Missing dimension: new-session path, step_selection None ──────

#[tokio::test]
async fn confirm_new_session_with_callback_no_step_selection() {
    let (mock, sm) = make_mock();
    let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
    let mut flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

    let tmp_dir = tempfile::tempdir().unwrap();
    let plan_path = tmp_dir.path().join("plan.md");
    std::fs::write(&plan_path, "# Plan\nStep 1").unwrap();
    let plan_path_str = plan_path.to_string_lossy().to_string();

    insert_plan_state(&mock, "parent-sess", &plan_path_str);

    let callback_invoked = Arc::new(AtomicBool::new(false));
    let ci = Arc::clone(&callback_invoked);
    let create_fn: CreateChildSessionFn = Arc::new(move |parent, _plan, _steps| {
        ci.store(true, Ordering::SeqCst);
        Box::pin(async move { Ok(format!("child-of-{parent}")) })
    });
    flow.set_create_child_session_fn(create_fn);

    let meta = PlanExecMetadata {
        plan_file_path: plan_path_str,
        step_selection: None,
        new_session: true,
        additional_instruction: Some("inject me".to_string()),
    };
    let id = flow.submit("parent-sess", meta).await;
    assert!(flow.confirm(&id).await);

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert!(callback_invoked.load(Ordering::SeqCst));
    let state = mock.state();
    let child_id = "child-of-parent-sess";
    assert_eq!(state.modes.get(child_id), Some(&SessionMode::Auto));

    // Additional instruction should be injected into child session.
    let child_msgs: Vec<&PendingMessage> = state
        .pending_messages
        .iter()
        .filter(|(sid, _)| sid == child_id)
        .map(|(_, m)| m)
        .collect();
    assert!(
        child_msgs.iter().any(|m| m.content == "inject me"),
        "additional instruction should be injected into child session"
    );
}

// ── Step 1.6: concurrent confirm same id only once ─────────────

#[tokio::test]
async fn concurrent_confirm_same_id_only_once() {
    let (_mock, sm) = make_mock();
    let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
    let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

    let meta = make_test_meta("/tmp/plan.md");
    let id = flow.submit("session-1", meta).await;

    // Launch two concurrent confirm calls on the same id.
    let flow1 = Arc::new(flow);
    let flow2 = Arc::clone(&flow1);
    let id1 = id.clone();
    let id2 = id.clone();
    let (r1, r2) = tokio::join!(flow1.confirm(&id1), flow2.confirm(&id2));

    // Exactly one should succeed.
    assert!(
        (r1 && !r2) || (!r1 && r2),
        "exactly one concurrent confirm should succeed: r1={}, r2={}",
        r1,
        r2
    );
}

// ── Step 1.3: Plan Mode → Auto pending transition ─────────────────────
// Validates: Plan Mode 下自然语言触发执行 → 退出 Plan + pending Auto
// (execution.md L120: session 标记 Auto Mode，切换不立即生效)

#[tokio::test]
async fn confirm_plan_mode_sets_pending_auto_not_immediate() {
    let (mock, sm) = make_mock();
    let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
    let flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

    // Set up session in Plan mode with a plan state
    let mut ps = PlanState::new();
    ps.plan_file_path = "/tmp/plan.md".to_string();
    mock.state()
        .plan_states
        .insert("session-plan".to_string(), ps);
    mock.state()
        .modes
        .insert("session-plan".to_string(), SessionMode::Plan);

    let meta = PlanExecMetadata {
        plan_file_path: "/tmp/plan.md".to_string(),
        step_selection: None,
        new_session: false,
        additional_instruction: None,
    };
    let id = flow.submit("session-plan", meta).await;

    assert!(flow.confirm(&id).await);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let state = mock.state();
    // Mode should be written to pending_modes (lazy), not modes (immediate)
    assert_eq!(
        state.pending_modes.get("session-plan"),
        Some(&SessionMode::Auto),
        "same-session confirm should set pending_session_mode to Auto (not immediate)"
    );
    // Immediate mode should NOT be changed (still Plan until lazy apply)
    assert_eq!(
        state.modes.get("session-plan"),
        Some(&SessionMode::Plan),
        "immediate mode should remain Plan until lazy application"
    );
    // Plan state should be updated to FinalPlan
    let ps = state.plan_states.get("session-plan").unwrap();
    assert_eq!(ps.phase, PlanPhase::FinalPlan);
}

// ── Step 1.3: New-session path → immediate Auto ────────────────────────
// Validates: 新 session 路径 → 立即 Auto (execution.md L134)
// This test anchors the contrast: new-session sets modes (immediate),
// same-session sets pending_modes (lazy).

#[tokio::test]
async fn confirm_new_session_sets_immediate_auto() {
    let (mock, sm) = make_mock();
    let on_notify: Arc<dyn Fn(PlanExecNotification) + Send + Sync> = Arc::new(|_| {});
    let mut flow = PlanExecConfirmFlow::new(sm, on_notify, tokio::runtime::Handle::current());

    let tmp_dir = tempfile::tempdir().unwrap();
    let plan_path = tmp_dir.path().join("plan.md");
    std::fs::write(&plan_path, "# Plan\nStep 1").unwrap();
    let plan_path_str = plan_path.to_string_lossy().to_string();

    insert_plan_state(&mock, "parent-sess", &plan_path_str);
    mock.state()
        .modes
        .insert("parent-sess".to_string(), SessionMode::Plan);

    let create_fn: CreateChildSessionFn =
        Arc::new(|parent, _plan, _steps| Box::pin(async move { Ok(format!("child-of-{parent}")) }));
    flow.set_create_child_session_fn(create_fn);

    let meta = PlanExecMetadata {
        plan_file_path: plan_path_str,
        step_selection: None,
        new_session: true,
        additional_instruction: None,
    };
    let id = flow.submit("parent-sess", meta).await;
    assert!(flow.confirm(&id).await);

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let state = mock.state();
    let child_id = "child-of-parent-sess";
    // New session: mode is set immediately (not pending)
    assert_eq!(
        state.modes.get(child_id),
        Some(&SessionMode::Auto),
        "new-session path should set mode to Auto immediately"
    );
    // pending_modes should NOT have an entry for child (it goes through set_session_mode)
    assert!(
        !state.pending_modes.contains_key(child_id),
        "new-session path should not use pending_mode (immediate application)"
    );
}
