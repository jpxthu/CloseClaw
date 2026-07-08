use super::background::PlanArchiveTask;
use std::fs;
use std::path::Path;

fn create_workspaces_root(dir: &Path) {
    fs::create_dir_all(dir.join("workspaces/agent1/user1/plans")).unwrap();
}

fn create_plan_file(dir: &Path, agent: &str, user: &str, name: &str, status: &str) {
    let plans_dir = dir.join("workspaces").join(agent).join(user).join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let path = plans_dir.join(name);
    let content = format!(
        "# Plan\n\n| 字段 | 值 |\n|------|-----|\n| 状态 | {status} |\n\n## Tasks\n\n- [x] Done\n"
    );
    fs::write(&path, content).unwrap();
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
