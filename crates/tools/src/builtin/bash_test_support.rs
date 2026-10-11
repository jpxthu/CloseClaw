//! Shared configurable `TaskManager` fake for the bash test suites.
//!
//! One fake replaces the per-file TaskManager mocks. Capabilities are the
//! union of the previous per-file mocks, nothing more:
//!
//! - task/call recording (`spawn_task` / `backgroundize_task` register the
//!   returned task; `backgroundize_task` arguments are recorded)
//! - backgroundize failure injection (`with_failing_backgroundize`,
//!   error text `boom`)
//! - configurable `max_execution_secs`
//! - optional pipe-reattachment assertions (`with_pipe_assertions`)
//!
//! `backgroundize_task` always kills and reaps the child so no process
//! outlives the test.

use closeclaw_common::tool_session::{KillHandle, ToolProgress, ToolSession};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One recorded `backgroundize_task` invocation.
#[derive(Debug, Clone)]
pub(crate) struct RecordedBackgroundize {
    pub(crate) command: String,
    pub(crate) is_backgrounded: bool,
    pub(crate) session_id: String,
}

#[derive(Default)]
struct FakeRecords {
    tasks: HashMap<String, closeclaw_common::BackgroundTask>,
    backgroundize_calls: Vec<RecordedBackgroundize>,
}

/// Configurable [`closeclaw_common::TaskManager`] fake shared by the bash
/// test suites. Cheap to clone; all clones observe the same records.
#[derive(Clone)]
pub(crate) struct FakeTaskManager {
    fail_backgroundize: bool,
    assert_pipes_reattached: bool,
    max_secs: u64,
    records: Arc<Mutex<FakeRecords>>,
}

impl FakeTaskManager {
    pub(crate) fn new() -> Self {
        Self {
            fail_backgroundize: false,
            assert_pipes_reattached: false,
            max_secs: 1800,
            records: Arc::new(Mutex::new(FakeRecords::default())),
        }
    }

    /// Builder switch: `backgroundize_task` fails with error text `boom`.
    pub(crate) fn with_failing_backgroundize(mut self) -> Self {
        self.fail_backgroundize = true;
        self
    }

    pub(crate) fn with_max_execution_secs(mut self, secs: u64) -> Self {
        self.max_secs = secs;
        self
    }

    /// Assert inside `backgroundize_task` that the child arrives with its
    /// stdout/stderr pipes reattached (the `backgroundize_child` contract).
    pub(crate) fn with_pipe_assertions(mut self) -> Self {
        self.assert_pipes_reattached = true;
        self
    }

    /// Sync observation of a recorded task by id.
    pub(crate) fn task(&self, task_id: &str) -> Option<closeclaw_common::BackgroundTask> {
        self.records.lock().unwrap().tasks.get(task_id).cloned()
    }

    pub(crate) fn backgroundize_calls(&self) -> Vec<RecordedBackgroundize> {
        self.records.lock().unwrap().backgroundize_calls.clone()
    }

    pub(crate) async fn is_running(&self, task_id: &str) -> bool {
        self.records
            .lock()
            .unwrap()
            .tasks
            .get(task_id)
            .is_some_and(|t| matches!(t.state, closeclaw_common::TaskState::Running { .. }))
    }
}

impl Default for FakeTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl closeclaw_common::TaskManager for FakeTaskManager {
    async fn spawn_task(
        &self,
        command: &str,
        cwd: &std::path::Path,
        is_backgrounded: bool,
        _session_id: &str,
    ) -> Result<closeclaw_common::BackgroundTask, closeclaw_common::BackgroundTaskError> {
        let task = closeclaw_common::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_common::TaskState::Running { is_backgrounded },
            output_path: cwd.join("output"),
        };
        self.records
            .lock()
            .unwrap()
            .tasks
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }

    async fn backgroundize_task(
        &self,
        mut child: tokio::process::Child,
        command: &str,
        is_backgrounded: bool,
        session_id: &str,
    ) -> Result<closeclaw_common::BackgroundTask, closeclaw_common::BackgroundTaskError> {
        if self.assert_pipes_reattached {
            assert!(
                child.stdout.is_some(),
                "child must arrive with its stdout pipe reattached"
            );
            assert!(
                child.stderr.is_some(),
                "child must arrive with its stderr pipe reattached"
            );
        }
        let _ = child.kill().await;
        if self.fail_backgroundize {
            return Err(closeclaw_common::BackgroundTaskError::SpawnFailed(
                "boom".into(),
            ));
        }
        let task = closeclaw_common::BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.to_string(),
            state: closeclaw_common::TaskState::Running { is_backgrounded },
            output_path: std::env::temp_dir().join("closeclaw-fake-task-output"),
        };
        let mut records = self.records.lock().unwrap();
        records.backgroundize_calls.push(RecordedBackgroundize {
            command: command.to_string(),
            is_backgrounded,
            session_id: session_id.to_string(),
        });
        records.tasks.insert(task.id.clone(), task.clone());
        Ok(task)
    }

    async fn kill_task(&self, task_id: &str) -> Result<(), closeclaw_common::BackgroundTaskError> {
        self.records.lock().unwrap().tasks.remove(task_id);
        Ok(())
    }

    async fn get_task(&self, task_id: &str) -> Option<closeclaw_common::BackgroundTask> {
        self.task(task_id)
    }

    async fn drain_notifications(&self) -> Vec<closeclaw_common::CompletionNotification> {
        vec![]
    }

    async fn list_running_tasks(&self) -> Vec<closeclaw_common::RunningTaskInfo> {
        vec![]
    }

    async fn cleanup_all_finished(&self, _session_id: &str) {}

    fn max_execution_secs(&self) -> u64 {
        self.max_secs
    }
}

/// Recording [`ToolSession`] fake shared by the fg-chain tests.
///
/// Captures `(call_id, ToolProgress)` pairs from `report_tool_progress`
/// so tests can assert which call_id the exec context's `session` /
/// `call_id` fields were consumed with on each foreground branch.
#[derive(Default)]
pub(crate) struct RecordingProgressSession {
    calls: Mutex<Vec<(String, ToolProgress)>>,
}

impl RecordingProgressSession {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Pair constructor: concrete handle backs the observation hooks,
    /// the trait object is handed to the code under test.
    pub(crate) fn pair() -> (Arc<Self>, Arc<dyn ToolSession>) {
        let concrete = Self::new();
        let trait_obj: Arc<dyn ToolSession> = concrete.clone();
        (concrete, trait_obj)
    }

    pub(crate) fn progress_calls(&self) -> Vec<(String, ToolProgress)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ToolSession for RecordingProgressSession {
    async fn register_tool_handle(&self, _call_id: String, _handle: Arc<dyn KillHandle>) {}

    async fn report_tool_progress(&self, call_id: &str, progress: ToolProgress) {
        self.calls
            .lock()
            .unwrap()
            .push((call_id.to_string(), progress));
    }
}
