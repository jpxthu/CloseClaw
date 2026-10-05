//! Behavior tests for the `TaskManager` contract: a mock implementing all
//! eight methods is usable as `Arc<dyn TaskManager>` (definition-side
//! composition contract), including sharing across concurrent tasks
//! (`Send + Sync` supertraits).
//!
//! 与实现侧用例的分工：`closeclaw-tasks` 的 `BackgroundTaskManager` 用例验证
//! 真实实现的行为（spawn/kill/通知排序等），gateway / tools 的用例验证各自
//! 装配点的 mock 经 `Arc<dyn TaskManager>` 注入；本文件是 trait 定义所在
//! crate 的契约验证——8 方法签名、组合后可作 trait 对象分发、超 trait 可
//! 跨任务共享。三组互不替代。
//!
//! 已被既有用例覆盖、此处不重复的维度：
//! - `NotificationPriority` 排序（Now > Next > Later）→
//!   `crates/tasks/src/background_tests.rs::test_notification_priority_traits`
//! - `RunningTaskInfo` / `CompletionNotification` 的构造与字段取值 →
//!   `crates/tasks/src/list_running_tasks_tests.rs` 与
//!   `crates/tasks/src/background_tests.rs`（经真实管理器构造并断言）

use super::*;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Fixed value returned by the mock's `max_execution_secs`.
const FIXED_MAX_SECS: u64 = 3600;

/// Mock implementing all eight [`TaskManager`] methods; every call is
/// recorded so the test can assert that each method dispatches through the
/// trait object (with its arguments forwarded) rather than being skipped.
struct MockTaskManager {
    calls: Mutex<Vec<String>>,
}

impl MockTaskManager {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn task(id: &str, command: &str, state: TaskState) -> BackgroundTask {
        BackgroundTask {
            id: id.to_string(),
            command: command.to_string(),
            state,
            output_path: PathBuf::from(format!("{id}.log")),
        }
    }
}

#[async_trait::async_trait]
impl TaskManager for MockTaskManager {
    async fn spawn_task(
        &self,
        command: &str,
        cwd: &Path,
        is_backgrounded: bool,
        session_id: &str,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        self.record(format!(
            "spawn_task|{command}|{}|{is_backgrounded}|{session_id}",
            cwd.display()
        ));
        Ok(Self::task(
            "t-spawn",
            command,
            TaskState::Running { is_backgrounded },
        ))
    }

    async fn backgroundize_task(
        &self,
        mut child: tokio::process::Child,
        command: &str,
        is_backgrounded: bool,
        session_id: &str,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        // Reap the short-lived child so the test leaves no process behind.
        let status = child.wait().await?;
        let exit_code = status.code().unwrap_or(-1);
        self.record(format!(
            "backgroundize_task|{command}|{is_backgrounded}|{session_id}|exit={exit_code}"
        ));
        Ok(Self::task(
            "t-bg",
            command,
            TaskState::Completed { exit_code },
        ))
    }

    async fn kill_task(&self, task_id: &str) -> Result<(), BackgroundTaskError> {
        self.record(format!("kill_task|{task_id}"));
        Ok(())
    }

    async fn get_task(&self, task_id: &str) -> Option<BackgroundTask> {
        self.record(format!("get_task|{task_id}"));
        Some(Self::task(
            task_id,
            "echo cached",
            TaskState::Running {
                is_backgrounded: false,
            },
        ))
    }

    async fn list_running_tasks(&self) -> Vec<RunningTaskInfo> {
        self.record("list_running_tasks".to_string());
        vec![RunningTaskInfo {
            task_id: "t-run".to_string(),
            command: "sleep 60".to_string(),
            elapsed_secs: 7,
        }]
    }

    async fn drain_notifications(&self) -> Vec<CompletionNotification> {
        self.record("drain_notifications".to_string());
        vec![CompletionNotification {
            task_id: "t-done".to_string(),
            command: "echo done".to_string(),
            state: TaskState::Completed { exit_code: 0 },
            output_path: PathBuf::from("t-done.log"),
            priority: NotificationPriority::Now,
            summary: "task t-done completed".to_string(),
            suggestion: None,
        }]
    }

    fn max_execution_secs(&self) -> u64 {
        self.record("max_execution_secs".to_string());
        FIXED_MAX_SECS
    }

    async fn cleanup_all_finished(&self, session_id: &str) {
        self.record(format!("cleanup_all_finished|{session_id}"));
    }
}

// ── 组合契约：8 方法 mock 可作 `Arc<dyn TaskManager>` 使用 ─────────────────

/// All eight methods must dispatch through `Arc<dyn TaskManager>` with their
/// arguments forwarded and their results handed back unchanged.
#[tokio::test]
async fn test_task_manager_mock_dispatches_all_eight_methods_via_dyn() {
    let mock = Arc::new(MockTaskManager::new());
    let tm: Arc<dyn TaskManager> = mock.clone();

    let spawned = tm
        .spawn_task("echo hi", Path::new("/work"), false, "sess-a")
        .await
        .expect("spawn_task must reach the mock");
    assert_eq!(spawned.id, "t-spawn");
    assert_eq!(
        spawned.state,
        TaskState::Running {
            is_backgrounded: false
        }
    );

    let child = tokio::process::Command::new("true")
        .spawn()
        .expect("spawn `true` for backgroundize_task");
    let backgroundized = tm
        .backgroundize_task(child, "true", true, "sess-a")
        .await
        .expect("backgroundize_task must reach the mock");
    assert_eq!(backgroundized.id, "t-bg");

    tm.kill_task("t-spawn").await.expect("kill_task");
    let fetched = tm
        .get_task("t-spawn")
        .await
        .expect("get_task must reach the mock");
    assert_eq!(fetched.id, "t-spawn");

    let running = tm.list_running_tasks().await;
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].task_id, "t-run");
    assert_eq!(running[0].command, "sleep 60");
    assert_eq!(running[0].elapsed_secs, 7);

    let notifications = tm.drain_notifications().await;
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].priority, NotificationPriority::Now);
    assert_eq!(notifications[0].task_id, "t-done");

    assert_eq!(tm.max_execution_secs(), FIXED_MAX_SECS);
    tm.cleanup_all_finished("sess-a").await;

    assert_eq!(
        mock.calls(),
        vec![
            "spawn_task|echo hi|/work|false|sess-a".to_string(),
            "backgroundize_task|true|true|sess-a|exit=0".to_string(),
            "kill_task|t-spawn".to_string(),
            "get_task|t-spawn".to_string(),
            "list_running_tasks".to_string(),
            "drain_notifications".to_string(),
            "max_execution_secs".to_string(),
            "cleanup_all_finished|sess-a".to_string(),
        ],
        "exactly the eight trait methods must dispatch through the trait object, in call order"
    );
}

/// `Send + Sync` supertraits: the trait object can be moved into concurrent
/// tasks and shared (daemon/gateway wire `Arc<dyn TaskManager>` across tasks).
#[tokio::test]
async fn test_task_manager_object_shared_across_concurrent_tasks() {
    let mock = Arc::new(MockTaskManager::new());
    let tm: Arc<dyn TaskManager> = mock.clone();

    let mut handles = Vec::new();
    for idx in 0..3u32 {
        let tm = Arc::clone(&tm);
        handles.push(tokio::spawn(async move {
            tm.kill_task(&format!("t-{idx}")).await.expect("kill_task");
            tm.max_execution_secs()
        }));
    }
    for handle in handles {
        assert_eq!(
            handle.await.expect("shared task must join"),
            FIXED_MAX_SECS,
            "every concurrent user must observe the same max_execution_secs"
        );
    }

    let mut kills: Vec<String> = mock
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("kill_task|"))
        .collect();
    kills.sort();
    assert_eq!(
        kills,
        vec![
            "kill_task|t-0".to_string(),
            "kill_task|t-1".to_string(),
            "kill_task|t-2".to_string(),
        ],
        "calls from concurrently shared tasks must all reach the mock"
    );
}
