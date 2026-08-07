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
#[path = "retention_tests.rs"]
mod retention_tests;
