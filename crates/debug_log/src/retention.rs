use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
use tracing::warn;

/// Errors that can occur during log retention operations.
#[derive(Debug, thiserror::Error)]
pub enum LogRetentionError {
    #[error("failed to read log directory: {0}")]
    ReadDir(#[source] std::io::Error),
    #[error("failed to delete log file: {0}")]
    Delete(#[source] std::io::Error),
    #[error("invalid date in filename: {0}")]
    InvalidDate(String),
}

/// Log retention policy — manages daily log file lifecycle.
///
/// Files are named `debug-{YYYY-MM-DD}.jsonl`. The retention policy
/// checks for expired files when rotation occurs (next-day first write)
/// and exposes a manual cleanup interface.
///
/// Only framework log files (`debug-*.jsonl`) are affected; other files
/// in the log directory are never touched.
#[derive(Debug, Clone)]
pub struct LogRetention {
    log_dir: PathBuf,
    retention_days: u32,
}

/// Parsed date from a framework log filename.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileDate {
    date: NaiveDate,
    path: PathBuf,
}

impl LogRetention {
    /// Create a new retention policy.
    pub fn new(log_dir: PathBuf, retention_days: u32) -> Self {
        Self {
            log_dir,
            retention_days,
        }
    }

    /// Cleanup log files older than the given date (exclusive).
    ///
    /// Returns the number of files deleted. Only files matching the
    /// `debug-*.jsonl` pattern are considered; other files are left untouched.
    pub fn cleanup_before(&self, date: NaiveDate) -> Result<usize, LogRetentionError> {
        let files = self.list_framework_files()?;
        let mut deleted = 0;

        for file_date in files {
            if file_date.date < date {
                if let Err(e) = std::fs::remove_file(&file_date.path) {
                    warn!(
                        path = %file_date.path.display(),
                        error = %e,
                        "failed to delete expired log file"
                    );
                } else {
                    deleted += 1;
                }
            }
        }

        Ok(deleted)
    }

    /// Run the retention check against the current date.
    ///
    /// Deletes files whose date is more than `retention_days` ago.
    /// Returns the number of files deleted.
    pub fn cleanup_expired(&self) -> Result<usize, LogRetentionError> {
        let today = Utc::now().date_naive();
        let cutoff = today - chrono::Duration::days(self.retention_days as i64);
        self.cleanup_before(cutoff)
    }

    /// List all framework log files (`debug-*.jsonl`) with parsed dates.
    fn list_framework_files(&self) -> Result<Vec<FileDate>, LogRetentionError> {
        let entries = std::fs::read_dir(&self.log_dir).map_err(LogRetentionError::ReadDir)?;

        let mut files = Vec::new();

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Some(date) = Self::parse_date_from_path(&path) {
                files.push(FileDate { date, path });
            }
        }

        Ok(files)
    }

    /// Parse `debug-{YYYY-MM-DD}.jsonl` filename into a `NaiveDate`.
    ///
    /// Returns `None` for files that don't match the framework naming pattern.
    fn parse_date_from_path(path: &Path) -> Option<NaiveDate> {
        let stem = path.file_stem()?.to_str()?;
        let ext = path.extension()?.to_str()?;

        if ext != "jsonl" {
            return None;
        }

        let prefix = "debug-";
        if !stem.starts_with(prefix) {
            return None;
        }

        let date_str = &stem[prefix.len()..];
        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;
        Some(date)
    }
}

#[cfg(test)]
mod tests {
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
}
