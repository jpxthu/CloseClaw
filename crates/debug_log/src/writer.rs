use std::path::PathBuf;

use chrono::{DateTime, NaiveDate};
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;

use crate::LogEvent;

/// Errors that can occur during log writing.
#[derive(Debug, thiserror::Error)]
pub enum LogWriterError {
    #[error("failed to open log file: {0}")]
    Open(#[source] std::io::Error),
    #[error("failed to serialize log event: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write log event: {0}")]
    Write(#[source] std::io::Error),
    #[error("failed to flush log file: {0}")]
    Flush(#[source] std::io::Error),
}

/// JSONL log writer that writes events to daily log files.
///
/// File naming: `debug-{YYYY-MM-DD}.jsonl`.
/// Each event is serialized and flushed immediately.
/// Write failures are logged via tracing and never block the caller.
///
/// Redaction is handled by `DebugLog` before events reach the writer.
#[derive(Debug)]
pub struct LogWriter {
    log_dir: PathBuf,
    current_date: Option<NaiveDate>,
    file: Option<File>,
}

impl LogWriter {
    /// Create a new writer. Log directory is created if it doesn't exist.
    pub async fn new(log_dir: PathBuf) -> Result<Self, LogWriterError> {
        tokio::fs::create_dir_all(&log_dir)
            .await
            .map_err(LogWriterError::Open)?;
        Ok(Self {
            log_dir,
            current_date: None,
            file: None,
        })
    }

    /// Write a log event to the current day's JSONL file.
    ///
    /// The event is serialized and flushed immediately.
    /// If any step fails, an error is reported via tracing and the
    /// method returns without panicking or blocking the caller.
    ///
    /// Redaction must be applied before calling this method.
    pub async fn write(&mut self, event: &LogEvent) -> Result<(), LogWriterError> {
        let today = DateTime::from_timestamp_millis(event.timestamp)
            .ok_or_else(|| {
                LogWriterError::Write(std::io::Error::other(format!(
                    "invalid timestamp millis: {}",
                    event.timestamp
                )))
            })?
            .date_naive();
        if self.current_date != Some(today) {
            self.rotate(today).await?;
        }

        let line = event.to_jsonl().map_err(LogWriterError::Serialize)?;
        let mut line_with_newline = line;
        line_with_newline.push('\n');

        let file = self
            .file
            .as_mut()
            .ok_or_else(|| LogWriterError::Write(std::io::Error::other("no active log file")))?;

        file.write_all(line_with_newline.as_bytes())
            .await
            .map_err(LogWriterError::Write)?;
        file.flush().await.map_err(LogWriterError::Flush)?;

        Ok(())
    }

    /// Open a new file for the given date and store the handle.
    async fn rotate(&mut self, date: NaiveDate) -> Result<(), LogWriterError> {
        let path = self.file_path(date);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(LogWriterError::Open)?;

        self.file = Some(file);
        self.current_date = Some(date);
        Ok(())
    }

    /// Return the file path for a given date.
    pub(crate) fn file_path(&self, date: NaiveDate) -> PathBuf {
        self.log_dir.join(format!("debug-{date}.jsonl"))
    }

    /// Return the current date being written to, if any.
    pub(crate) fn current_date(&self) -> Option<NaiveDate> {
        self.current_date
    }
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod writer_tests;
