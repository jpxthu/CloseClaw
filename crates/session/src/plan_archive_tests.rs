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

#[test]
fn test_is_completed_all_failed() {
    let content = "## Tasks\n\n- [!] Step 1\n- [!] Step 2\n";
    assert!(is_completed_plan(content));
}

#[test]
fn test_is_completed_done_failed_skipped_mix() {
    let content = "## Tasks\n\n- [x] Done\n- [!] Failed\n- [~] Skipped\n";
    assert!(is_completed_plan(content));
}

// --- Error path ---

#[test]
fn test_is_completed_has_failed() {
    let content = "## Tasks\n\n- [x] Step 1\n- [!] Step 2 failed\n";
    assert!(is_completed_plan(content));
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
fn test_is_completed_single_failed() {
    let content = "## Tasks\n\n- [!] Only step\n";
    assert!(is_completed_plan(content));
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
fn test_archiver_archives_failed_step_plan() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = create_plan_file(
        dir.path(),
        "failed.md",
        &["- [x] Step 1", "- [!] Step 2 broke"],
    );
    set_old_mtime(&path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path.exists());
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

    // Has failed, old → should archive (failed is terminal)
    let path4 = plans_dir.join("failed-old.md");
    fs::write(&path4, "# P4\n\n## Tasks\n\n- [x] A\n- [!] B\n").unwrap();
    set_old_mtime(&path4);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 2);
    assert!(!path1.exists());
    assert!(path2.exists());
    assert!(path3.exists());
    assert!(!path4.exists());
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

// ===========================================================================
// Access timestamp archival tests
// ===========================================================================

/// Helper: create a completed plan with an access timestamp set to the
/// given number of seconds ago.
fn create_completed_plan_with_access_ts(
    dir: &Path,
    name: &str,
    seconds_ago: i64,
) -> std::path::PathBuf {
    let path = create_plan_file(dir, name, &["- [x] Step 1", "- [x] Step 2"]);
    // Set access timestamp to seconds_ago in the past
    let past_time = chrono::Utc::now() - chrono::Duration::seconds(seconds_ago);
    let marker = format!("<!-- accessed: {} -->", past_time.to_rfc3339());
    let mut content = fs::read_to_string(&path).unwrap();
    // Insert marker after title heading
    let insert_pos = content.find("\n# ").map(|p| p + 1).unwrap_or(0);
    content.insert_str(insert_pos, &format!("{marker}\n"));
    fs::write(&path, &content).unwrap();
    path
}

/// Helper: create a completed plan with no access timestamp (legacy).
fn create_completed_plan_legacy(dir: &Path, name: &str) -> std::path::PathBuf {
    create_plan_file(dir, name, &["- [x] Step 1", "- [x] Step 2"])
}

#[test]
fn test_archiver_access_ts_over_threshold_archives() {
    let dir = tempfile::TempDir::new().unwrap();
    // 10 days ago access timestamp → should archive with 7-day threshold
    let path = create_completed_plan_with_access_ts(dir.path(), "old-access.md", 10 * 86400);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path.exists());
    assert!(dir.path().join("plans/archive/old-access.md").exists());
}

#[test]
fn test_archiver_access_ts_under_threshold_skips() {
    let dir = tempfile::TempDir::new().unwrap();
    // 3 days ago access timestamp → should NOT archive with 7-day threshold
    let path = create_completed_plan_with_access_ts(dir.path(), "recent-access.md", 3 * 86400);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_no_access_ts_fallback_to_mtime() {
    let dir = tempfile::TempDir::new().unwrap();
    // Legacy plan with no access timestamp, old mtime → should archive via mtime
    let path = create_completed_plan_legacy(dir.path(), "legacy-old.md");
    set_old_mtime(&path);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path.exists());
}

#[test]
fn test_archiver_no_access_ts_new_mtime_skips() {
    let dir = tempfile::TempDir::new().unwrap();
    // Legacy plan with no access timestamp, recent mtime → should NOT archive
    let path = create_completed_plan_legacy(dir.path(), "legacy-new.md");
    // mtime is recent by default (just created)

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_active_plan_with_old_access_ts_skips() {
    let dir = tempfile::TempDir::new().unwrap();
    // Active plan (pending step) with old access timestamp → should NOT archive
    let path = create_plan_file(
        dir.path(),
        "active-old.ts.md",
        &["- [x] Step 1", "- [ ] Step 2"],
    );
    let past_time = chrono::Utc::now() - chrono::Duration::days(30);
    let marker = format!("<!-- accessed: {} -->", past_time.to_rfc3339());
    let mut content = fs::read_to_string(&path).unwrap();
    let insert_pos = content.find("\n# ").map(|p| p + 1).unwrap_or(0);
    content.insert_str(insert_pos, &format!("{marker}\n"));
    fs::write(&path, &content).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_completed_exactly_at_threshold_skips() {
    let dir = tempfile::TempDir::new().unwrap();
    // Access timestamp just under threshold (6d 23h 59m) → should NOT archive
    let path = create_plan_file(dir.path(), "boundary.md", &["- [x] Done"]);
    let boundary_time = chrono::Utc::now() - chrono::Duration::hours(6 * 24 + 23);
    let marker = format!("<!-- accessed: {} -->", boundary_time.to_rfc3339());
    let mut content = fs::read_to_string(&path).unwrap();
    let insert_pos = content.find("\n# ").map(|p| p + 1).unwrap_or(0);
    content.insert_str(insert_pos, &format!("{marker}\n"));
    fs::write(&path, &content).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

#[test]
fn test_archiver_access_ts_over_just_past_threshold_archives() {
    let dir = tempfile::TempDir::new().unwrap();
    // Access timestamp 7 days + 1 second ago → should archive
    let path = create_plan_file(dir.path(), "just-over.md", &["- [x] Done"]);
    let past_time = chrono::Utc::now() - chrono::Duration::days(7) - chrono::Duration::seconds(1);
    let marker = format!("<!-- accessed: {} -->", past_time.to_rfc3339());
    let mut content = fs::read_to_string(&path).unwrap();
    let insert_pos = content.find("\n# ").map(|p| p + 1).unwrap_or(0);
    content.insert_str(insert_pos, &format!("{marker}\n"));
    fs::write(&path, &content).unwrap();

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 1);
    assert!(!path.exists());
}

#[test]
fn test_archiver_multiple_plans_mixed_access_ts_and_legacy() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();

    // Plan A: completed, access ts 10 days ago → archive
    let path_a = plans_dir.join("a.md");
    fs::write(&path_a, "# A\n\n## Tasks\n\n- [x] Done\n").unwrap();
    let ts_a = chrono::Utc::now() - chrono::Duration::days(10);
    let marker_a = format!("<!-- accessed: {} -->", ts_a.to_rfc3339());
    let mut content_a = fs::read_to_string(&path_a).unwrap();
    let pos_a = content_a.find("\n# ").map(|p| p + 1).unwrap_or(0);
    content_a.insert_str(pos_a, &format!("{marker_a}\n"));
    fs::write(&path_a, &content_a).unwrap();

    // Plan B: completed, access ts 2 days ago → skip
    let path_b = plans_dir.join("b.md");
    fs::write(&path_b, "# B\n\n## Tasks\n\n- [x] Done\n").unwrap();
    let ts_b = chrono::Utc::now() - chrono::Duration::days(2);
    let marker_b = format!("<!-- accessed: {} -->", ts_b.to_rfc3339());
    let mut content_b = fs::read_to_string(&path_b).unwrap();
    let pos_b = content_b.find("\n# ").map(|p| p + 1).unwrap_or(0);
    content_b.insert_str(pos_b, &format!("{marker_b}\n"));
    fs::write(&path_b, &content_b).unwrap();

    // Plan C: completed, no access ts, old mtime → archive (mtime fallback)
    let path_c = plans_dir.join("c.md");
    fs::write(&path_c, "# C\n\n## Tasks\n\n- [x] Done\n").unwrap();
    set_old_mtime(&path_c);

    // Plan D: active (pending), no access ts, old mtime → skip
    let path_d = plans_dir.join("d.md");
    fs::write(&path_d, "# D\n\n## Tasks\n\n- [x] Step 1\n- [ ] Step 2\n").unwrap();
    set_old_mtime(&path_d);

    let archiver = PlanArchiver::new(7);
    let count = archiver.archive(dir.path()).unwrap();
    assert_eq!(count, 2); // a + c archived
    assert!(!path_a.exists());
    assert!(path_b.exists());
    assert!(!path_c.exists());
    assert!(path_d.exists());
}

#[test]
fn test_archiver_content_preserved_with_access_ts() {
    let dir = tempfile::TempDir::new().unwrap();
    let content =
        "# My Plan\n\n<!-- accessed: 2020-01-01T00:00:00Z -->\n\n## Tasks\n\n- [x] Do thing\n";
    let plans_dir = dir.path().join("plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let path = plans_dir.join("content-test.md");
    fs::write(&path, content).unwrap();

    let archiver = PlanArchiver::new(7);
    archiver.archive(dir.path()).unwrap();

    let dest = dir.path().join("plans/archive/content-test.md");
    assert!(dest.exists());
    let archived_content = fs::read_to_string(&dest).unwrap();
    assert!(archived_content.contains("# My Plan"));
    assert!(archived_content.contains("<!-- accessed:"));
    assert!(archived_content.contains("- [x] Do thing"));
}
