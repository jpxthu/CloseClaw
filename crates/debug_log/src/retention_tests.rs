use super::*;
use chrono::NaiveDate;
use std::fs;

fn create_test_file(dir: &Path, name: &str) {
    fs::write(dir.join(name), "test content\n").unwrap();
}

#[test]
fn test_cleanup_before_deletes_old_files() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    // Create files with dates that would be parsed from names.
    create_test_file(tmp.path(), "debug-2026-01-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-06-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-01.jsonl");

    let cutoff = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
    let deleted = retention.cleanup_before(cutoff).unwrap();

    assert_eq!(deleted, 2);
    assert!(!tmp.path().join("debug-2026-01-01.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-06-01.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-01.jsonl").exists());
}

#[test]
fn test_cleanup_before_preserves_recent_files() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-08-05.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-06.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-07.jsonl");

    let cutoff = NaiveDate::from_ymd_opt(2026, 8, 6).unwrap();
    let deleted = retention.cleanup_before(cutoff).unwrap();

    assert_eq!(deleted, 1);
    assert!(!tmp.path().join("debug-2026-08-05.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-06.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-07.jsonl").exists());
}

#[test]
fn test_cleanup_before_does_not_delete_non_framework_files() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-01-01.jsonl");
    create_test_file(tmp.path(), "app-2026-01-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-01-01.log");
    create_test_file(tmp.path(), "readme.txt");

    let cutoff = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let deleted = retention.cleanup_before(cutoff).unwrap();

    // Only the debug JSONL file should be deleted.
    assert_eq!(deleted, 1);
    assert!(!tmp.path().join("debug-2026-01-01.jsonl").exists());
    assert!(tmp.path().join("app-2026-01-01.jsonl").exists());
    assert!(tmp.path().join("debug-2026-01-01.log").exists());
    assert!(tmp.path().join("readme.txt").exists());
}

#[test]
fn test_cleanup_before_no_files_to_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-08-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-07.jsonl");

    // Cutoff is before all files — nothing to delete.
    let cutoff = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
    let deleted = retention.cleanup_before(cutoff).unwrap();

    assert_eq!(deleted, 0);
    assert!(tmp.path().join("debug-2026-08-01.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-07.jsonl").exists());
}

#[test]
fn test_cleanup_before_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let deleted = retention.cleanup_before(cutoff).unwrap();

    assert_eq!(deleted, 0);
}

#[test]
fn test_parse_date_from_path_valid() {
    let path = PathBuf::from("/tmp/debug-2026-08-07.jsonl");
    let date = LogRetention::parse_date_from_path(&path).unwrap();
    assert_eq!(date, NaiveDate::from_ymd_opt(2026, 8, 7).unwrap());
}

#[test]
fn test_parse_date_from_path_invalid_format() {
    let path = PathBuf::from("/tmp/debug-not-a-date.jsonl");
    assert!(LogRetention::parse_date_from_path(&path).is_none());
}

#[test]
fn test_parse_date_from_path_wrong_prefix() {
    let path = PathBuf::from("/tmp/app-2026-08-07.jsonl");
    assert!(LogRetention::parse_date_from_path(&path).is_none());
}

#[test]
fn test_parse_date_from_path_wrong_extension() {
    let path = PathBuf::from("/tmp/debug-2026-08-07.log");
    assert!(LogRetention::parse_date_from_path(&path).is_none());
}

#[test]
fn test_cleanup_before_boundary_date_exclusive() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-07-31.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-01.jsonl");

    // Cutoff is 2026-08-01 — only files strictly before are deleted.
    let cutoff = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let deleted = retention.cleanup_before(cutoff).unwrap();

    assert_eq!(deleted, 1);
    assert!(!tmp.path().join("debug-2026-07-31.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-01.jsonl").exists());
}

#[test]
fn test_cleanup_before_subdirectories_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    // Create a subdirectory whose name would be parseable as a log file.
    let subdir = tmp.path().join("debug-2026-01-01.jsonl");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(subdir.join("nested.jsonl"), "content").unwrap();

    // Cutoff is far in the past — the subdirectory matches the name pattern
    // but is_file() returns false, so it won't be listed or deleted.
    let cutoff = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let deleted = retention.cleanup_before(cutoff).unwrap();

    assert_eq!(deleted, 0);
    assert!(subdir.exists());
}

// --- cleanup_all tests ---

#[test]
fn test_cleanup_all_deletes_all_files() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-01-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-06-15.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-07.jsonl");

    let deleted = retention.cleanup_all().unwrap();

    assert_eq!(deleted, 3);
    assert!(!tmp.path().join("debug-2026-01-01.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-06-15.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-08-07.jsonl").exists());
}

#[test]
fn test_cleanup_all_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    let deleted = retention.cleanup_all().unwrap();

    assert_eq!(deleted, 0);
}

#[test]
fn test_cleanup_all_preserves_non_framework_files() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-01-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-07.jsonl");
    create_test_file(tmp.path(), "app-2026-01-01.jsonl");
    create_test_file(tmp.path(), "readme.txt");

    let deleted = retention.cleanup_all().unwrap();

    assert_eq!(deleted, 2);
    assert!(!tmp.path().join("debug-2026-01-01.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-08-07.jsonl").exists());
    assert!(tmp.path().join("app-2026-01-01.jsonl").exists());
    assert!(tmp.path().join("readme.txt").exists());
}

// --- cleanup_range tests ---

#[test]
fn test_cleanup_range_deletes_files_in_range() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-01-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-06-15.jsonl");
    create_test_file(tmp.path(), "debug-2026-07-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-07.jsonl");

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 7, 31).unwrap();
    let deleted = retention.cleanup_range(from, to).unwrap();

    assert_eq!(deleted, 2);
    assert!(tmp.path().join("debug-2026-01-01.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-06-15.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-07-01.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-07.jsonl").exists());
}

#[test]
fn test_cleanup_range_boundary_inclusive() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-06-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-07-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-01.jsonl");

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
    let deleted = retention.cleanup_range(from, to).unwrap();

    assert_eq!(deleted, 2);
    assert!(!tmp.path().join("debug-2026-06-01.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-07-01.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-01.jsonl").exists());
}

#[test]
fn test_cleanup_range_single_day() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-06-30.jsonl");
    create_test_file(tmp.path(), "debug-2026-07-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-07-02.jsonl");

    let from = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
    let deleted = retention.cleanup_range(from, to).unwrap();

    assert_eq!(deleted, 1);
    assert!(tmp.path().join("debug-2026-06-30.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-07-01.jsonl").exists());
    assert!(tmp.path().join("debug-2026-07-02.jsonl").exists());
}

#[test]
fn test_cleanup_range_cross_month() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-06-30.jsonl");
    create_test_file(tmp.path(), "debug-2026-07-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-07-15.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-01.jsonl");

    let from = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
    let deleted = retention.cleanup_range(from, to).unwrap();

    assert_eq!(deleted, 3);
    assert!(!tmp.path().join("debug-2026-06-30.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-07-01.jsonl").exists());
    assert!(!tmp.path().join("debug-2026-07-15.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-01.jsonl").exists());
}

#[test]
fn test_cleanup_range_no_files_in_range() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-01-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-08-07.jsonl");

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 7, 31).unwrap();
    let deleted = retention.cleanup_range(from, to).unwrap();

    assert_eq!(deleted, 0);
    assert!(tmp.path().join("debug-2026-01-01.jsonl").exists());
    assert!(tmp.path().join("debug-2026-08-07.jsonl").exists());
}

#[test]
fn test_cleanup_range_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 7, 31).unwrap();
    let deleted = retention.cleanup_range(from, to).unwrap();

    assert_eq!(deleted, 0);
}

#[test]
fn test_cleanup_range_preserves_non_framework_files() {
    let tmp = tempfile::tempdir().unwrap();
    let retention = LogRetention::new(tmp.path().into(), 7);

    create_test_file(tmp.path(), "debug-2026-07-01.jsonl");
    create_test_file(tmp.path(), "app-2026-07-01.jsonl");
    create_test_file(tmp.path(), "debug-2026-07-01.log");

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let deleted = retention.cleanup_range(from, to).unwrap();

    assert_eq!(deleted, 1);
    assert!(!tmp.path().join("debug-2026-07-01.jsonl").exists());
    assert!(tmp.path().join("app-2026-07-01.jsonl").exists());
    assert!(tmp.path().join("debug-2026-07-01.log").exists());
}
