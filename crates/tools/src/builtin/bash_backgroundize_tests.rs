//! Direct behavior tests for the backgroundize chain.
//!
//! `backgroundize_child` / `auto_backgroundize_foreground` are the
//! terminal builders of both backgrounding paths (manual signal and
//! auto-background timeout). The result-field dimension is already
//! covered indirectly via `handle_foreground_result` tests in
//! `bash_tests.rs`; these tests lock the chain's own contract directly:
//!
//! - `by_user` selects the manual vs auto result shape
//! - the task is registered in the `TaskManager` with the exact
//!   `session_id` and `is_backgrounded=true` passed down the chain
//! - the returned task id is identical to the registered task id and
//!   the id exposed in the result data
//! - `backgroundize_task` failure propagates as a mapped error string /
//!   `ForegroundOutcome::Failed`
//! - the timeout path forwards `ForegroundContext.session_id` unchanged

use super::*;
use std::path::PathBuf;

/// One recorded `backgroundize_task` invocation.
#[derive(Debug, Clone)]
struct RecordedBackgroundize {
    command: String,
    is_backgrounded: bool,
    session_id: String,
}

/// Shared recorder inspected by the tests after the call under test.
struct BgRecorder {
    calls: std::sync::Mutex<Vec<RecordedBackgroundize>>,
    tasks: std::sync::Mutex<Vec<closeclaw_tasks::BackgroundTask>>,
}

impl BgRecorder {
    fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            tasks: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<RecordedBackgroundize> {
        self.calls.lock().unwrap().clone()
    }

    fn task(&self, task_id: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.id == task_id)
            .cloned()
    }
}

/// Mock `TaskManager` that records `backgroundize_task` arguments.
///
/// When `fail` is true, `backgroundize_task` returns an error so the
/// error-propagation path of the chain can be exercised. The child is
/// always killed and reaped immediately: the mock asserts the
/// registration chain, not process liveness, and this keeps the test
/// process free of lingering children.
struct RecordingBgManager {
    recorder: std::sync::Arc<BgRecorder>,
    fail: bool,
}

impl RecordingBgManager {
    fn ok() -> (std::sync::Arc<BgRecorder>, Self) {
        let recorder = std::sync::Arc::new(BgRecorder::new());
        let mgr = Self {
            recorder: std::sync::Arc::clone(&recorder),
            fail: false,
        };
        (recorder, mgr)
    }

    fn failing() -> Self {
        Self {
            recorder: std::sync::Arc::new(BgRecorder::new()),
            fail: true,
        }
    }
}

/// Kill and reap a mock-held child so no process outlives the test.
async fn reap_child(mut child: tokio::process::Child) {
    let _ = child.kill().await;
}

#[async_trait::async_trait]
impl closeclaw_tasks::TaskManager for RecordingBgManager {
    async fn spawn_task(
        &self,
        _command: &str,
        _cwd: &std::path::Path,
        _is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        Err(closeclaw_tasks::BackgroundTaskError::SpawnFailed(
            "not used".into(),
        ))
    }

    async fn backgroundize_task(
        &self,
        child: tokio::process::Child,
        command: &str,
        is_backgrounded: bool,
        session_id: &str,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        // Lock the pipe reattachment behavior: the production chain
        // (`backgroundize_child`) must reattach the taken stdout/stderr
        // handles before handing the child to the manager. Without the
        // reattach in `backgroundize_child`, these assertions fail.
        assert!(
            child.stdout.is_some(),
            "child must arrive with its stdout pipe reattached"
        );
        assert!(
            child.stderr.is_some(),
            "child must arrive with its stderr pipe reattached"
        );
        reap_child(child).await;
        if self.fail {
            return Err(closeclaw_tasks::BackgroundTaskError::SpawnFailed(
                "boom".into(),
            ));
        }
        let task = closeclaw_tasks::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_tasks::TaskState::Running { is_backgrounded },
            // Placeholder only, never written to disk: this fake never
            // performs output I/O; the backgroundize chain under test
            // only forwards the path into the result payload.
            output_path: PathBuf::from("/nonexistent/closeclaw-fake-task-output"),
        };
        self.recorder
            .calls
            .lock()
            .unwrap()
            .push(RecordedBackgroundize {
                command: command.to_string(),
                is_backgrounded,
                session_id: session_id.to_string(),
            });
        self.recorder.tasks.lock().unwrap().push(task.clone());
        Ok(task)
    }

    async fn kill_task(&self, _task_id: &str) -> Result<(), closeclaw_tasks::BackgroundTaskError> {
        // Deliberately a no-op: this suite never exercises the kill
        // chain, so there is nothing meaningful to record or remove.
        Ok(())
    }

    async fn get_task(&self, task_id: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        self.recorder.task(task_id)
    }

    async fn drain_notifications(&self) -> Vec<closeclaw_tasks::CompletionNotification> {
        vec![]
    }

    async fn list_running_tasks(&self) -> Vec<closeclaw_tasks::RunningTaskInfo> {
        vec![]
    }

    async fn cleanup_all_finished(&self, _session_id: &str) {}

    fn max_execution_secs(&self) -> u64 {
        1800
    }
}

/// Build `ChildHandles` from a freshly spawned child, mirroring the
/// production hand-off (pipes taken out of the child first).
fn make_handles(command: &str, cwd: &str) -> ChildHandles {
    let mut child = spawn_sh_command(command, cwd).expect("spawn child");
    ChildHandles {
        stdout_handle: child.stdout.take(),
        stderr_handle: child.stderr.take(),
        child,
    }
}

// ---------------------------------------------------------------------------
// backgroundize_child: result shape per `by_user`
// ---------------------------------------------------------------------------

/// `by_user=true` → manual background result (`backgroundedByUser`),
/// task registered with the given session_id, ids all consistent.
#[tokio::test]
async fn test_backgroundize_child_by_user_builds_manual_result() {
    let (recorder, mgr) = RecordingBgManager::ok();
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(mgr);
    let tmp = tempfile::TempDir::new().unwrap();

    let handles = make_handles("true", tmp.path().to_str().unwrap());
    let (result, returned_task_id) =
        backgroundize_child(handles, "true", &bg_trait, true, "sess-manual")
            .await
            .expect("backgroundize_child should succeed");

    // Manual shape: backgroundedByUser=true, no auto flag.
    assert_eq!(
        result.data["backgroundedByUser"],
        serde_json::json!(true),
        "by_user=true must build the manual result with backgroundedByUser=true"
    );
    assert!(
        result.data["assistantAutoBackgrounded"].is_null(),
        "manual result must not set assistantAutoBackgrounded"
    );
    let result_task_id = result.data["backgroundTaskId"]
        .as_str()
        .expect("manual result must expose backgroundTaskId as a string");
    assert!(!result_task_id.is_empty());
    assert!(
        result.data["outputPath"].is_string(),
        "manual result must expose outputPath"
    );

    // Returned id == registered id == id in the result data.
    assert_eq!(
        returned_task_id, result_task_id,
        "returned task id must match the id exposed in the result"
    );
    let task = recorder
        .task(&returned_task_id)
        .expect("task must be registered in the TaskManager");
    assert_eq!(task.id, returned_task_id);
    assert_eq!(task.command, "true");
    assert!(matches!(
        task.state,
        closeclaw_tasks::TaskState::Running {
            is_backgrounded: true
        }
    ));

    // Registration chain: command / is_backgrounded=true / session_id
    // forwarded unchanged.
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1, "exactly one backgroundize_task call");
    assert_eq!(calls[0].command, "true");
    assert!(
        calls[0].is_backgrounded,
        "backgroundize chain must register is_backgrounded=true"
    );
    assert_eq!(
        calls[0].session_id, "sess-manual",
        "session_id must be forwarded unchanged to the TaskManager"
    );
}

/// `by_user=false` → auto background result (`assistantAutoBackgrounded`).
#[tokio::test]
async fn test_backgroundize_child_auto_builds_auto_result() {
    let (recorder, mgr) = RecordingBgManager::ok();
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(mgr);
    let tmp = tempfile::TempDir::new().unwrap();

    let handles = make_handles("true", tmp.path().to_str().unwrap());
    let (result, returned_task_id) =
        backgroundize_child(handles, "true", &bg_trait, false, "sess-auto")
            .await
            .expect("backgroundize_child should succeed");

    // Auto shape: assistantAutoBackgrounded=true, no user flag.
    assert_eq!(
        result.data["assistantAutoBackgrounded"],
        serde_json::json!(true),
        "by_user=false must build the auto result with assistantAutoBackgrounded=true"
    );
    assert!(
        result.data["backgroundedByUser"].is_null(),
        "auto result must not set backgroundedByUser"
    );
    let result_task_id = result.data["backgroundTaskId"]
        .as_str()
        .expect("auto result must expose backgroundTaskId as a string");
    assert_eq!(
        returned_task_id, result_task_id,
        "returned task id must match the id exposed in the result"
    );
    assert!(
        recorder.task(&returned_task_id).is_some(),
        "task must be registered in the TaskManager"
    );
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].session_id, "sess-auto");
}

// ---------------------------------------------------------------------------
// backgroundize_child / auto_backgroundize_foreground: error propagation
// ---------------------------------------------------------------------------

/// `backgroundize_task` failure → mapped error string from
/// `backgroundize_child`.
#[tokio::test]
async fn test_backgroundize_child_maps_manager_error() {
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(RecordingBgManager::failing());
    let tmp = tempfile::TempDir::new().unwrap();

    let handles = make_handles("true", tmp.path().to_str().unwrap());
    let err = backgroundize_child(handles, "true", &bg_trait, true, "sess-err")
        .await
        .expect_err("failing manager must propagate an error");

    assert!(
        err.starts_with("failed to backgroundize command: "),
        "error must carry the backgroundize-chain prefix, got: {}",
        err
    );
    assert!(
        err.contains("boom"),
        "error must include the manager's own error text, got: {}",
        err
    );
}

/// `auto_backgroundize_foreground` wraps Ok into
/// `ForegroundOutcome::AutoBackground` with consistent ids.
#[tokio::test]
async fn test_auto_backgroundize_foreground_ok_maps_to_auto_background() {
    let (recorder, mgr) = RecordingBgManager::ok();
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(mgr);
    let tmp = tempfile::TempDir::new().unwrap();

    let handles = make_handles("true", tmp.path().to_str().unwrap());
    let outcome =
        auto_backgroundize_foreground(handles, "true", &bg_trait, false, "sess-wrap").await;

    let (result, task_id) = match outcome {
        ForegroundOutcome::AutoBackground(r, id) => (r, id),
        other => panic!("expected AutoBackground, got: {:?}", other),
    };
    assert_eq!(
        result.data["assistantAutoBackgrounded"],
        serde_json::json!(true),
        "wrapped result must keep the auto shape"
    );
    assert_eq!(
        result.data["backgroundTaskId"],
        serde_json::json!(task_id),
        "outcome task id must match the id in the result data"
    );
    assert!(
        recorder.task(&task_id).is_some(),
        "task must be registered under the outcome's task id"
    );
    assert_eq!(recorder.calls()[0].session_id, "sess-wrap");
}

/// `auto_backgroundize_foreground` wraps Err into
/// `ForegroundOutcome::Failed` with the mapped message.
#[tokio::test]
async fn test_auto_backgroundize_foreground_err_maps_to_failed() {
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(RecordingBgManager::failing());
    let tmp = tempfile::TempDir::new().unwrap();

    let handles = make_handles("true", tmp.path().to_str().unwrap());
    let outcome =
        auto_backgroundize_foreground(handles, "true", &bg_trait, true, "sess-fail").await;

    let msg = match outcome {
        ForegroundOutcome::Failed(m) => m,
        other => panic!("expected Failed, got: {:?}", other),
    };
    assert!(
        msg.starts_with("failed to backgroundize command: ") && msg.contains("boom"),
        "Failed outcome must carry the mapped backgroundize error, got: {}",
        msg
    );
}

// ---------------------------------------------------------------------------
// Timeout path: ForegroundContext.session_id forwarding
// ---------------------------------------------------------------------------

/// `handle_timeout_expiry` (non-force-terminate) auto-backgroundizes and
/// forwards `ctx.session_id` unchanged to the registration chain.
#[tokio::test]
async fn test_handle_timeout_expiry_auto_background_forwards_ctx_session_id() {
    let (recorder, mgr) = RecordingBgManager::ok();
    let bg_trait: Arc<dyn closeclaw_tasks::TaskManager> = Arc::new(mgr);
    let tmp = tempfile::TempDir::new().unwrap();

    let mut child = spawn_sh_command("sleep 5", tmp.path().to_str().unwrap()).expect("spawn sleep");
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let handles = ChildHandles {
        child,
        stdout_handle,
        stderr_handle,
    };

    let ctx = ForegroundContext {
        bg_manager: &bg_trait,
        manual_bg_signal: None,
        session: None,
        call_id: None,
        session_id: "sess-timeout",
        force_terminate: false,
    };
    let outcome = handle_timeout_expiry(handles, "sleep 5", &ctx).await;

    let (result, task_id) = match outcome {
        ForegroundOutcome::AutoBackground(r, id) => (r, id),
        other => panic!("expected AutoBackground on timeout, got: {:?}", other),
    };
    assert_eq!(
        result.data["assistantAutoBackgrounded"],
        serde_json::json!(true),
        "timeout auto-background must produce the auto result shape"
    );
    assert_eq!(
        result.data["backgroundTaskId"],
        serde_json::json!(task_id),
        "result id must match the outcome task id"
    );
    assert!(
        recorder.task(&task_id).is_some(),
        "task must be registered under the outcome task id"
    );
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].is_backgrounded,
        "timeout path must register is_backgrounded=true"
    );
    assert_eq!(
        calls[0].session_id, "sess-timeout",
        "ForegroundContext.session_id must be forwarded unchanged on timeout"
    );
}
