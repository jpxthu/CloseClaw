use super::background::PlanArchiveTask;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn create_workspaces_root(dir: &Path) {
    fs::create_dir_all(dir.join("workspaces/agent1/user1/plans")).unwrap();
}

fn create_plan_file(dir: &Path, agent: &str, user: &str, name: &str, status: &str) {
    let plans_dir = dir.join("workspaces").join(agent).join(user).join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let path = plans_dir.join(name);
    let step_marker = match status {
        "draft" => "- [ ] Pending",
        "executing" => "- [-] InProgress",
        "completed" => "- [x] Done",
        _ => "- [ ] Pending",
    };
    let content = format!("# Plan\n\n## Tasks\n\n{step_marker}\n");
    fs::write(&path, content).unwrap();
}

/// Drop guard for a hung task future: flips the shared flag when the
/// task future is dropped, proving the abort branch actually ran.
struct AbortGuard(Arc<AtomicBool>);
impl Drop for AbortGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn test_run_once_archives_old_completed_plans() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());
    create_plan_file(
        dir.path(),
        "agent1",
        "user1",
        "old-completed.md",
        "completed",
    );

    // Set mtime to 10 days ago
    let path = dir
        .path()
        .join("workspaces/agent1/user1/plans/old-completed.md");
    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);
    task.run_once().await;

    // Should be archived
    let archived = dir
        .path()
        .join("workspaces/agent1/user1/plans/archive/old-completed.md");
    assert!(archived.exists(), "old completed plan should be archived");
    assert!(
        !path.exists(),
        "original plan should be moved out of plans/"
    );
}

#[tokio::test]
async fn test_run_once_skips_recent_completed_plans() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());
    create_plan_file(
        dir.path(),
        "agent1",
        "user1",
        "recent-completed.md",
        "completed",
    );

    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);
    task.run_once().await;

    // Should NOT be archived (too new)
    let path = dir
        .path()
        .join("workspaces/agent1/user1/plans/recent-completed.md");
    assert!(
        path.exists(),
        "recent completed plan should not be archived"
    );
}

#[tokio::test]
async fn test_run_once_skips_draft_plans() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());
    create_plan_file(dir.path(), "agent1", "user1", "old-draft.md", "draft");

    let path = dir
        .path()
        .join("workspaces/agent1/user1/plans/old-draft.md");
    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);
    task.run_once().await;

    assert!(path.exists(), "draft plan should not be archived");
}

#[tokio::test]
async fn test_run_once_handles_multiple_agents_and_users() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());
    fs::create_dir_all(dir.path().join("workspaces/agent2/user2/plans")).unwrap();

    // agent1/user1: old completed → should archive
    create_plan_file(dir.path(), "agent1", "user1", "plan1.md", "completed");
    let path1 = dir.path().join("workspaces/agent1/user1/plans/plan1.md");
    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path1, filetime::FileTime::from_system_time(old_time)).unwrap();

    // agent2/user2: old completed → should archive
    create_plan_file(dir.path(), "agent2", "user2", "plan2.md", "completed");
    let path2 = dir.path().join("workspaces/agent2/user2/plans/plan2.md");
    filetime::set_file_mtime(&path2, filetime::FileTime::from_system_time(old_time)).unwrap();

    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);
    task.run_once().await;

    assert!(!path1.exists(), "agent1/user1 plan should be archived");
    assert!(dir
        .path()
        .join("workspaces/agent1/user1/plans/archive/plan1.md")
        .exists());
    assert!(!path2.exists(), "agent2/user2 plan should be archived");
    assert!(dir
        .path()
        .join("workspaces/agent2/user2/plans/archive/plan2.md")
        .exists());
}

#[tokio::test]
async fn test_run_once_no_workspaces_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    // No workspaces/ directory at all
    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);
    // Should not panic
    task.run_once().await;
}

#[tokio::test]
async fn test_run_once_empty_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());
    // plans/ exists but is empty
    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);
    task.run_once().await;
}

#[tokio::test]
async fn test_run_once_custom_threshold() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());
    create_plan_file(dir.path(), "agent1", "user1", "three-days.md", "completed");

    // Set mtime to 3 days ago
    let path = dir
        .path()
        .join("workspaces/agent1/user1/plans/three-days.md");
    let three_days = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(three_days)).unwrap();

    // Threshold 7 days → should NOT archive
    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);
    task.run_once().await;
    assert!(
        path.exists(),
        "3-day-old plan should not archive with 7-day threshold"
    );

    // Threshold 2 days → should archive
    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 2);
    task.run_once().await;
    assert!(
        !path.exists(),
        "3-day-old plan should archive with 2-day threshold"
    );
}

#[tokio::test]
async fn test_run_once_skips_executing_plans() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());
    create_plan_file(dir.path(), "agent1", "user1", "executing.md", "executing");

    let path = dir
        .path()
        .join("workspaces/agent1/user1/plans/executing.md");
    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);
    task.run_once().await;

    assert!(path.exists(), "executing plan should not be archived");
}

#[tokio::test]
async fn test_shutdown_signal_stops_task() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());
    create_plan_file(dir.path(), "agent1", "user1", "plan.md", "completed");

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);

    // Spawn the task and immediately send shutdown
    let handle = tokio::spawn(async move {
        task.run(shutdown_rx).await;
    });

    // Give it a moment to start, then shutdown
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let _ = shutdown_tx.send(());

    // Task should exit cleanly
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "task should exit cleanly after shutdown signal"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.4: PlanArchiveTask grace period tests
// ═══════════════════════════════════════════════════════════════════════════

/// ARCHIVE_GRACE_PERIOD_SECS is 10 seconds (aligned with design doc
/// "统一最长 10 秒").
#[test]
fn test_archive_grace_period_is_10_secs() {
    assert_eq!(super::background::ARCHIVE_GRACE_PERIOD_SECS, 10);
}

/// When no task is running at shutdown, wait_grace_period returns
/// immediately.
#[tokio::test]
async fn test_plan_archive_grace_period_none_returns_immediately() {
    let start = tokio::time::Instant::now();
    super::background::PlanArchiveTask::wait_grace_period(None).await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "None task should return immediately, took {:?}",
        elapsed
    );
}

/// When the running task completes within the grace period,
/// wait_grace_period does NOT abort it.
#[tokio::test]
async fn test_plan_archive_grace_period_no_abort_when_completed() {
    let completing_task = tokio::task::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    });

    let start = tokio::time::Instant::now();
    super::background::PlanArchiveTask::wait_grace_period(Some(completing_task)).await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "should complete quickly when task finishes within grace, took {:?}",
        elapsed
    );
}

/// When a task exceeds the grace period, wait_grace_period aborts it.
///
/// Runs on a paused tokio clock (`start_paused`): the hanging task is
/// parked on `std::future::pending` (registers no timer), so the paused
/// clock's auto-advance drives only the grace-period `sleep` inside
/// `wait_grace_period`. The full timing chain — grace period expires →
/// task abort (future dropped) — is still genuinely executed and
/// asserted while real wall-clock cost collapses to milliseconds.
///
/// Abort proof is deterministic by waiter mode: the timing and the
/// drop-drain run inside a `waiter` future on the same runtime, with
/// `hanging_task` moved into it. `waiter.await` returning
/// unconditionally guarantees the hanging future's `Drop` guard has
/// executed — `JoinHandle::abort()` only schedules the cancellation on
/// the runtime and never drops the future synchronously, and a task
/// marked cancelled stays on its runtime until it is polled to
/// completion, so the in-waiter yield drain terminates through
/// scheduler semantics, not poll-count luck.
#[tokio::test(start_paused = true, flavor = "current_thread")]
#[serial_test::serial]
async fn test_plan_archive_grace_period_abort_on_timeout() {
    let aborted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&aborted);
    let hanging_task = tokio::task::spawn(async move {
        let _guard = AbortGuard(flag);
        std::future::pending::<()>().await;
    });

    // Waiter: owns the timing, the `wait_grace_period` call and the
    // abort-drop drain on the same runtime as `hanging_task`, which is
    // moved into it. `waiter.await` returning is itself the completion
    // guarantee that the guard has run — no poll-count heuristics.
    let waiter_flag = Arc::clone(&aborted);
    let waiter = async move {
        // Virtual-clock start: `wait_grace_period` must consume exactly the
        // grace constant before aborting — any early return (or waiting on
        // a completion that can never happen here) shifts `elapsed`.
        let start = tokio::time::Instant::now();
        super::background::PlanArchiveTask::wait_grace_period(Some(hanging_task)).await;
        let elapsed = start.elapsed();

        // Paused clock is deterministic: the select can only exit via the
        // grace sleep, so elapsed is exactly the grace period. This also
        // pins the abort branch as the only feasible exit before the
        // drain loop below is allowed to run.
        assert_eq!(
            elapsed,
            tokio::time::Duration::from_secs(super::background::ARCHIVE_GRACE_PERIOD_SECS),
            "must wait out exactly the grace period before aborting, took {elapsed:?}"
        );

        // Abort proof: drain the runtime until the cancelled task has
        // actually been polled to its `Drop`. `abort()` scheduled the
        // cancellation on this very runtime, so each `yield_now` lets
        // the scheduler make progress on it.
        while !waiter_flag.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    };

    waiter.await;

    assert!(
        aborted.load(Ordering::SeqCst),
        "hanging task future must have been dropped by the abort branch"
    );
}

/// PlanArchiveTask: signal → running task completes within grace →
/// clean exit (no abort). Tests the full signal → grace → exit path.
#[tokio::test]
async fn test_plan_archive_signal_grace_clean_exit() {
    let dir = tempfile::TempDir::new().unwrap();
    create_workspaces_root(dir.path());

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let task = PlanArchiveTask::new(dir.path().to_path_buf(), 7);

    let handle = tokio::spawn(async move {
        task.run(shutdown_rx).await;
    });

    // Wait for task to enter the loop, then signal shutdown.
    // No sweep is running (empty plans dir), so exit should be
    // immediate (no grace period wait).
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let _ = shutdown_tx.send(());

    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "task should exit within grace period after shutdown signal"
    );
    assert!(result.unwrap().is_ok(), "task should not panic");
}

/// PlanArchiveTask: hung task → grace period expires → abort.
/// Tests the abort path when a sweep doesn't finish in time.
///
/// Runs on a paused tokio clock (`start_paused`): the hung task is
/// parked on `std::future::pending` (registers no timer), so the
/// paused clock's auto-advance drives only the grace-period `sleep`.
/// The full select chain — hung task vs. grace sleep → abort — is
/// still genuinely executed and asserted while real wall-clock cost
/// collapses to milliseconds.
///
/// Abort proof is deterministic by waiter mode: the select and the
/// drop-drain run inside a `waiter` future on the same runtime, with
/// `hang_handle` moved into it. `waiter.await` returning
/// unconditionally guarantees the hung future's `Drop` guard has
/// executed — `JoinHandle::abort()` only schedules the cancellation on
/// the runtime and never drops the future synchronously, and a task
/// marked cancelled stays on its runtime until it is polled to
/// completion, so the in-waiter yield drain terminates through
/// scheduler semantics, not poll-count luck.
#[tokio::test(start_paused = true, flavor = "current_thread")]
#[serial_test::serial]
async fn test_plan_archive_hung_task_aborted_after_grace() {
    let aborted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&aborted);
    // Spawn a task that never exits (simulates a hung sweep)
    let mut hang_handle = tokio::task::spawn(async move {
        let _guard = AbortGuard(flag);
        std::future::pending::<()>().await;
    });

    // Waiter: owns the select (grace sleep vs. hung task) and the
    // abort-drop drain on the same runtime as `hang_handle`, which is
    // moved into it. `waiter.await` returning is itself the completion
    // guarantee that the guard has run — no poll-count heuristics.
    let waiter_flag = Arc::clone(&aborted);
    let waiter = async move {
        // Reproduce the grace period + abort select pattern from
        // PlanArchiveTask. Virtual-clock start: the sleep must consume
        // exactly the grace constant — any other outcome shifts `elapsed`.
        let grace = tokio::time::Duration::from_secs(super::background::ARCHIVE_GRACE_PERIOD_SECS);
        let start = tokio::time::Instant::now();

        tokio::select! {
            _ = &mut hang_handle => {
                // Task completed — should not happen
                panic!("hung task should not complete");
            }
            _ = tokio::time::sleep(grace) => {
                // Grace period expired — abort
                hang_handle.abort();
            }
        }

        // Paused clock is deterministic: the select can only exit via the
        // grace sleep, so elapsed is exactly the grace period. This also
        // pins the abort branch as the only feasible exit before the
        // drain loop below is allowed to run.
        let elapsed = start.elapsed();
        assert_eq!(
            elapsed, grace,
            "should wait out exactly the grace period before aborting, took {elapsed:?}"
        );

        // Abort proof: drain the runtime until the cancelled task has
        // actually been polled to its `Drop`. `abort()` scheduled the
        // cancellation on this very runtime, so each `yield_now` lets
        // the scheduler make progress on it.
        while !waiter_flag.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    };

    waiter.await;

    assert!(
        aborted.load(Ordering::SeqCst),
        "hung task future must have been dropped by the abort"
    );
}

/// Verify the two-layer wait pattern: inner grace period (per-task)
/// and outer join timeout (Phase 3). The inner grace is 10s for
/// PlanArchiveTask; the outer join timeout in phase_3_background_stop
/// is also 10s. This test verifies the inner layer works independently:
/// a sweep finishing before the grace expires takes the clean completion
/// branch of `wait_grace_period`'s select (Ok path, no abort), regardless
/// of the outer join timeout layer.
///
/// Runs on a paused tokio clock (`start_paused`): both the slow task's
/// 5s sleep and the grace period's 10s sleep register timers, so the
/// auto-advancing clock fires the 5s one first — the task completes
/// before the grace expires and the select resolves through the
/// completion branch. The full timing chain — task finishes at 5s,
/// before the 10s grace, without abort — is genuinely executed while
/// real wall-clock cost collapses to milliseconds (same approach as
/// `test_plan_archive_grace_period_abort_on_timeout`).
#[tokio::test(start_paused = true, flavor = "current_thread")]
#[serial_test::serial]
async fn test_plan_archive_inner_grace_independent_of_outer_timeout() {
    // Named task duration for both the sleep and the expected elapsed —
    // symmetric with the abort-side case expressing its expected duration
    // via a constant (`ARCHIVE_GRACE_PERIOD_SECS`).
    const TASK_SECS: u64 = 5;

    // A task that completes in 5s (within inner grace of 10s). The flag
    // is only set after the sleep, so it proves natural completion — an
    // abort during the sleep would skip the store entirely.
    let completed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&completed);
    let slow_task = tokio::task::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(TASK_SECS)).await;
        flag.store(true, Ordering::SeqCst);
    });

    let start = tokio::time::Instant::now();
    super::background::PlanArchiveTask::wait_grace_period(Some(slow_task)).await;
    let elapsed = start.elapsed();

    // Paused clock is deterministic: the 5s task timer fires strictly
    // before the 10s grace timer, so the select can only exit via the
    // task completion branch at exactly 5s. The grace branch (abort at
    // 10s) is the only alternative and would double the elapsed time.
    assert_eq!(
        elapsed,
        tokio::time::Duration::from_secs(TASK_SECS),
        "select must exit via task completion branch at exactly {TASK_SECS}s, took {elapsed:?}"
    );

    // Completion-branch proof: the task ran to natural completion and
    // was never aborted mid-flight.
    assert!(
        completed.load(Ordering::SeqCst),
        "task must complete naturally (no abort) within the grace period"
    );
}
