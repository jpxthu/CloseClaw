use super::plan_archive::*;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Helper: create a plan file with step markers in the Tasks section
// ---------------------------------------------------------------------------

fn create_plan_file(dir: &Path, name: &str, steps: &[&str]) -> std::path::PathBuf {
    let plans_dir = dir.join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let path = plans_dir.join(name);
    let tasks_block = steps.join("\n");
    let content = format!("# Test Plan\n\n## Tasks\n\n{tasks_block}\n");
    fs::write(&path, content).unwrap();
    path
}

// ===========================================================================
// parse_step_markers tests
// ===========================================================================

#[test]
fn test_parse_step_markers_all_done() {
    let content = "## Tasks\n\n- [x] Step one\n- [x] Step two\n";
    let states = parse_step_markers(content);
    assert_eq!(states.len(), 2);
    assert!(states.iter().all(|s| *s == StepState::Done));
}

#[test]
fn test_parse_step_markers_mixed_done_and_skipped() {
    let content = "## Tasks\n\n- [x] Done\n- [~] Skipped\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::Done, StepState::Skipped]);
}

#[test]
fn test_parse_step_markers_pending() {
    let content = "## Tasks\n\n- [ ] Not started\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::Pending]);
}

#[test]
fn test_parse_step_markers_in_progress() {
    let content = "## Tasks\n\n- [-] Doing it\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::InProgress]);
}

#[test]
fn test_parse_step_markers_failed() {
    let content = "## Tasks\n\n- [!] Broken\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::Failed]);
}

#[test]
fn test_parse_step_markers_asterisk_prefix() {
    let content = "## Tasks\n\n* [x] Star bullet\n* [ ] Pending star\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::Done, StepState::Pending]);
}

#[test]
fn test_parse_step_markers_indented() {
    let content = "## Tasks\n\n  - [x] Indented done\n    - [ ] Deep pending\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::Done, StepState::Pending]);
}

#[test]
fn test_parse_step_markers_no_markers() {
    let content = "## Tasks\n\nJust some plain text.\n";
    let states = parse_step_markers(content);
    assert!(states.is_empty());
}

#[test]
fn test_parse_step_markers_unknown_marker_ignored() {
    let content = "## Tasks\n\n- [x] Done\n- [z] Unknown\n- [~] Skipped\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::Done, StepState::Skipped]);
}

#[test]
fn test_parse_step_markers_single_done() {
    let content = "- [x] Only step\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::Done]);
}

#[test]
fn test_parse_step_markers_single_pending() {
    let content = "- [ ] Only step\n";
    let states = parse_step_markers(content);
    assert_eq!(states, vec![StepState::Pending]);
}

// ===========================================================================
// is_completed_plan tests
// ===========================================================================

// --- Normal path ---

#[test]
fn test_is_completed_all_done() {
    let content = "## Tasks\n\n- [x] Step 1\n- [x] Step 2\n";
    assert!(is_completed_plan(content));
}

#[test]
fn test_is_completed_done_and_skipped() {
    let content = "## Tasks\n\n- [x] Step 1\n- [~] Step 2\n";
    assert!(is_completed_plan(content));
}

#[test]
fn test_is_completed_all_skipped() {
    let content = "## Tasks\n\n- [~] Step 1\n- [~] Step 2\n";
    assert!(is_completed_plan(content));
}

// --- Error path ---

#[test]
fn test_is_not_completed_has_failed() {
    let content = "## Tasks\n\n- [x] Step 1\n- [!] Step 2 failed\n";
    assert!(!is_completed_plan(content));
}

#[test]
fn test_is_not_completed_has_in_progress() {
    let content = "## Tasks\n\n- [x] Step 1\n- [-] Step 2 in progress\n";
    assert!(!is_completed_plan(content));
}

#[test]
fn test_is_not_completed_has_pending() {
    let content = "## Tasks\n\n- [x] Step 1\n- [ ] Step 2 not started\n";
    assert!(!is_completed_plan(content));
}

#[test]
fn test_is_not_completed_single_in_progress() {
    let content = "## Tasks\n\n- [-] Only step\n";
    assert!(!is_completed_plan(content));
}

// --- Boundary values ---

#[test]
fn test_is_not_completed_empty_tasks() {
    let content = "## Tasks\n\nNothing here.\n";
    assert!(!is_completed_plan(content));
}

#[test]
fn test_is_not_completed_no_tasks_section() {
    let content = "# Plan\n\nSome content without tasks.\n";
    assert!(!is_completed_plan(content));
}

#[test]
fn test_is_completed_single_done() {
    let content = "## Tasks\n\n- [x] Only step\n";
    assert!(is_completed_plan(content));
}

#[test]
fn test_is_not_completed_single_pending() {
    let content = "## Tasks\n\n- [ ] Only step\n";
    assert!(!is_completed_plan(content));
}

#[test]
fn test_is_not_completed_single_failed() {
    let content = "## Tasks\n\n- [!] Only step\n";
    assert!(!is_completed_plan(content));
}

// --- State transition: simulating plan from active to complete ---

#[test]
fn test_transition_pending_to_in_progress() {
    let s1 = "## Tasks\n\n- [ ] Step 1\n";
    let s2 = "## Tasks\n\n- [-] Step 1\n";
    assert!(!is_completed_plan(s1));
    assert!(!is_completed_plan(s2));
}

#[test]
fn test_transition_in_progress_to_done() {
    let s1 = "## Tasks\n\n- [-] Step 1\n";
    let s2 = "## Tasks\n\n- [x] Step 1\n";
    assert!(!is_completed_plan(s1));
    assert!(is_completed_plan(s2));
}

#[test]
fn test_transition_full_lifecycle() {
    // Simulate a plan with 3 steps going through full lifecycle
    let pending = "## Tasks\n\n- [ ] A\n- [ ] B\n- [ ] C\n";
    let progress = "## Tasks\n\n- [x] A\n- [-] B\n- [ ] C\n";
    let almost = "## Tasks\n\n- [x] A\n- [x] B\n- [ ] C\n";
    let done = "## Tasks\n\n- [x] A\n- [x] B\n- [x] C\n";

    assert!(!is_completed_plan(pending));
    assert!(!is_completed_plan(progress));
    assert!(!is_completed_plan(almost));
    assert!(is_completed_plan(done));
}

// --- Format compatibility ---

#[test]
fn test_format_asterisk_bullets() {
    let content = "## Tasks\n\n* [x] One\n* [x] Two\n";
    assert!(is_completed_plan(content));
}

#[test]
fn test_format_mixed_dash_and_asterisk() {
    let content = "## Tasks\n\n- [x] Dash\n* [x] Star\n";
    assert!(is_completed_plan(content));
}

#[test]
fn test_format_indented_markers() {
    let content = "## Tasks\n\n  - [x] Indented one\n    - [x] Deep two\n";
    assert!(is_completed_plan(content));
}

// ===========================================================================
// Integration: PlanArchiver::archive with step-marker-based plans
// ===========================================================================

fn set_old_mtime(path: &Path) {
    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(path, filetime::FileTime::from_system_time(old_time)).unwrap();
}

#[test]
fn test_archiver_archives_completed_step_plan() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "done.md", &["- [x] Step 1", "- [x] Step 2"]);
    set_old_mtime(&path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path.exists());
    assert!(
        dir.path().join("plans/archive/done.md").exists(),
        "archived file should be in plans/archive/"
    );
}

#[test]
fn test_archiver_skips_active_step_plan() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "active.md", &["- [x] Step 1", "- [ ] Step 2"]);
    set_old_mtime(&path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_skips_failed_step_plan() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(
        dir.path(),
        "failed.md",
        &["- [x] Step 1", "- [!] Step 2 broke"],
    );
    set_old_mtime(&path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_archives_done_and_skipped_mix() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(dir.path(), "mix.md", &["- [x] Done", "- [~] Skipped"]);
    set_old_mtime(&path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path.exists());
}

#[test]
fn test_archiver_skips_empty_tasks_plan() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let path = plans_dir.join("empty.md");
    fs::write(&path, "# Plan\n\n## Tasks\n\nNothing.\n").unwrap();
    set_old_mtime(&path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_content_intact_after_archive() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = "# My Plan\n\n## Tasks\n\n- [x] Do thing\n- [x] Done\n";

    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let path = plans_dir.join("content-test.md");
    fs::write(&path, content).unwrap();
    set_old_mtime(&path);

    let archiver = PlanArchiver::new(7);
    archiver.archive(dir.path()).unwrap();

    let dest = dir.path().join("plans/archive/content-test.md");
    assert!(dest.exists());
    let archived_content = fs::read_to_string(&dest).unwrap();
    assert_eq!(archived_content, content);
}

#[test]
fn test_archiver_multiple_files_mixed_step_status() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();

    // All done, old → should archive
    let path1 = plans_dir.join("done-old.md");
    fs::write(&path1, "# P1\n\n## Tasks\n\n- [x] A\n- [x] B\n").unwrap();
    set_old_mtime(&path1);

    // All done, new → should NOT archive (not old enough)
    let path2 = plans_dir.join("done-new.md");
    fs::write(&path2, "# P2\n\n## Tasks\n\n- [x] A\n").unwrap();

    // Has pending, old → should NOT archive
    let path3 = plans_dir.join("pending-old.md");
    fs::write(&path3, "# P3\n\n## Tasks\n\n- [x] A\n- [ ] B\n").unwrap();
    set_old_mtime(&path3);

    // Has failed, old → should NOT archive
    let path4 = plans_dir.join("failed-old.md");
    fs::write(&path4, "# P4\n\n## Tasks\n\n- [x] A\n- [!] B\n").unwrap();
    set_old_mtime(&path4);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path1.exists());
    assert!(path2.exists());
    assert!(path3.exists());
    assert!(path4.exists());
}

#[test]
fn test_archiver_no_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_archiver_skips_non_md_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();

    let txt_path = plans_dir.join("notes.txt");
    fs::write(&txt_path, "not a plan").unwrap();
    set_old_mtime(&txt_path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(txt_path.exists());
}

#[test]
fn test_archiver_skips_archive_subdir() {
    let dir = tempfile::TempDir::new().unwrap();
    let archive_dir = dir.path().join("plans/archive");
    fs::create_dir_all(&archive_dir).unwrap();

    let archived_path = archive_dir.join("already-archived.md");
    fs::write(&archived_path, "# Old\n\n## Tasks\n\n- [x] Done\n").unwrap();
    set_old_mtime(&archived_path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(archived_path.exists());
}

#[test]
fn test_archive_error_display() {
    let err = ArchiveError::InvalidPath(std::path::PathBuf::from("/bad"));
    assert!(err.to_string().contains("invalid path"));

    let err = ArchiveError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"));
    assert!(err.to_string().contains("I/O error"));
}
