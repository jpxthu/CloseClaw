use super::plan_archive::*;
use std::fs;
use std::path::Path;

fn create_plan_file(dir: &Path, name: &str, status: &str) -> std::path::PathBuf {
    let plans_dir = dir.join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let path = plans_dir.join(name);
    let content = format!(
        "# Test Plan\n\n| 字段 | 值 |\n|------|-----|\n| 状态 | {status} |\n\n## Tasks\n\n- [x] Done\n"
    );
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn test_parse_plan_status_completed() {
    let content = "# Title\n\n| 字段 | 值 |\n| 状态 | completed |\n";
    assert_eq!(parse_plan_status(content).as_deref(), Some("completed"));
}

#[test]
fn test_parse_plan_status_draft() {
    let content = "# Title\n\n| 字段 | 值 |\n| 状态 | draft |\n";
    assert_eq!(parse_plan_status(content).as_deref(), Some("draft"));
}

#[test]
fn test_parse_plan_status_missing() {
    let content = "# Title\n\nNo status here.\n";
    assert_eq!(parse_plan_status(content), None);
}

#[test]
fn test_parse_plan_status_executing() {
    let content = "# Title\n\n| 状态 | executing |\n";
    assert_eq!(parse_plan_status(content).as_deref(), Some("executing"));
}

#[test]
fn test_parse_plan_status_completed_with_surrounding_content() {
    let content =
        "# Title\n\n| 字段 | 值 |\n|------|-----|\n| 状态 | completed |\n\n## Context\n\nDone.\n";
    assert_eq!(parse_plan_status(content).as_deref(), Some("completed"));
}

#[test]
fn test_archiver_skips_when_not_old_enough() {
    let dir = tempfile::TempDir::new().unwrap();
    create_plan_file(dir.path(), "plan1.md", "completed");

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(
        dir.path().join("plans/plan1.md").exists(),
        "file should remain in plans/"
    );
}

#[test]
fn test_archiver_moves_when_old_enough() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "plan-old.md", "completed");

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(
        !dir.path().join("plans/plan-old.md").exists(),
        "file should be moved out of plans/"
    );
    assert!(
        dir.path().join("plans/archive/plan-old.md").exists(),
        "file should be in plans/archive/"
    );
}

#[test]
fn test_archiver_skips_draft_status() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "draft-plan.md", "draft");

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(
        dir.path().join("plans/draft-plan.md").exists(),
        "draft plan should not be archived"
    );
}

#[test]
fn test_archiver_no_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_archiver_content_intact_after_archive() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = "# My Plan\n\n| 状态 | completed |\n\n## Tasks\n\n- [x] Do thing\n";

    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let path = plans_dir.join("content-test.md");
    fs::write(&path, &content).unwrap();

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let archiver = PlanArchiver::new(7);
    archiver.archive(dir.path()).unwrap();

    let dest = dir.path().join("plans/archive/content-test.md");
    assert!(dest.exists());
    let archived_content = fs::read_to_string(&dest).unwrap();
    assert_eq!(archived_content, content);
}

#[test]
fn test_archiver_skips_non_md_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);

    let txt_path = plans_dir.join("notes.txt");
    fs::write(&txt_path, "not a plan").unwrap();
    filetime::set_file_mtime(&txt_path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(txt_path.exists(), "non-md files should not be moved");
}

#[test]
fn test_archiver_skips_archive_subdir() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    let archive_dir = plans_dir.join("archive");
    fs::create_dir_all(&archive_dir).unwrap();

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);

    let archived_path = archive_dir.join("already-archived.md");
    fs::write(&archived_path, "# Old\n\n| 状态 | completed |\n").unwrap();
    filetime::set_file_mtime(
        &archived_path,
        filetime::FileTime::from_system_time(old_time),
    )
    .unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(archived_path.exists());
}

#[test]
fn test_archiver_multiple_files_mixed_status() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);

    // completed, old → should archive
    let path1 = plans_dir.join("completed-old.md");
    fs::write(&path1, "# P1\n\n| 状态 | completed |\n").unwrap();
    filetime::set_file_mtime(&path1, filetime::FileTime::from_system_time(old_time)).unwrap();

    // completed, new → should NOT archive
    let path2 = plans_dir.join("completed-new.md");
    fs::write(&path2, "# P2\n\n| 状态 | completed |\n").unwrap();

    // draft, old → should NOT archive
    let path3 = plans_dir.join("draft-old.md");
    fs::write(&path3, "# P3\n\n| 状态 | draft |\n").unwrap();
    filetime::set_file_mtime(&path3, filetime::FileTime::from_system_time(old_time)).unwrap();

    // executing, old → should NOT archive
    let path4 = plans_dir.join("executing-old.md");
    fs::write(&path4, "# P4\n\n| 状态 | executing |\n").unwrap();
    filetime::set_file_mtime(&path4, filetime::FileTime::from_system_time(old_time)).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path1.exists());
    assert!(path2.exists());
    assert!(path3.exists());
    assert!(path4.exists());
}

#[test]
fn test_convenience_function() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "convenience.md", "completed");

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let count = archive_completed_plans(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(dir.path().join("plans/archive/convenience.md").exists());
}

#[test]
fn test_convenience_function_with_threshold() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "threshold.md", "completed");

    let three_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(three_days_ago)).unwrap();

    // Threshold 7 days → should NOT archive
    let count = archive_completed_plans_with_threshold(dir.path(), 7).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());

    // Threshold 2 days → should archive
    let count = archive_completed_plans_with_threshold(dir.path(), 2).unwrap();
    assert_eq!(count, 1);
    assert!(!path.exists());
}

#[test]
fn test_archiver_skips_executing_status() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "exec-plan.md", "executing");

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_skips_paused_status() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "paused-plan.md", "paused");

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_empty_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_archiver_default_threshold() {
    // Verify default threshold works: a plan 8 days old should archive
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "default-thresh.md", "completed");

    let eight_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(eight_days_ago)).unwrap();

    let archiver = PlanArchiver::with_defaults();
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path.exists());
}

#[test]
fn test_archive_error_display() {
    let err = ArchiveError::InvalidPath(std::path::PathBuf::from("/bad"));
    assert!(err.to_string().contains("invalid path"));

    let err = ArchiveError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"));
    assert!(err.to_string().contains("I/O error"));
}
