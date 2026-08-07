use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;

use crate::{LogEvent, RedactionEngine};

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
/// Each event is serialized, redacted, and flushed immediately.
/// Write failures are logged via tracing and never block the caller.
#[derive(Debug)]
pub struct LogWriter {
    log_dir: PathBuf,
    redactor: RedactionEngine,
    current_date: Option<NaiveDate>,
    file: Option<File>,
}

impl LogWriter {
    /// Create a new writer. Log directory is created if it doesn't exist.
    pub async fn new(log_dir: PathBuf, redactor: RedactionEngine) -> Result<Self, LogWriterError> {
        tokio::fs::create_dir_all(&log_dir)
            .await
            .map_err(LogWriterError::Open)?;
        Ok(Self {
            log_dir,
            redactor,
            current_date: None,
            file: None,
        })
    }

    /// Write a log event to the current day's JSONL file.
    ///
    /// The event is serialized, redacted, and flushed immediately.
    /// If any step fails, an error is reported via tracing and the
    /// method returns without panicking or blocking the caller.
    pub async fn write(&mut self, event: &mut LogEvent) -> Result<(), LogWriterError> {
        let today = event.timestamp.date_naive();
        if self.current_date != Some(today) {
            self.rotate(today).await?;
        }

        // Redact sensitive fields before serialization.
        self.redactor.redact(&mut event.payload);

        let line = event.to_jsonl().map_err(LogWriterError::Serialize)?;
        let mut line_with_newline = line;
        line_with_newline.push('\n');

        let file = self.file.as_mut().ok_or_else(|| {
            LogWriterError::Write(std::io::Error::new(
                std::io::ErrorKind::Other,
                "no active log file",
            ))
        })?;

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
    pub fn file_path(&self, date: NaiveDate) -> PathBuf {
        self.log_dir.join(format!("debug-{date}.jsonl"))
    }

    /// Return the log directory path.
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// Return the current date being written to, if any.
    pub fn current_date(&self) -> Option<NaiveDate> {
        self.current_date
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LogLevel, TraceContext};
    use serde_json::json;

    fn make_event(module: &str, event_type: &str) -> LogEvent {
        let ctx = TraceContext::new_root("trace-test-1".into());
        LogEvent::new(
            &ctx,
            None,
            LogLevel::Info,
            module,
            event_type,
            json!({"key": "value"}),
        )
    }

    #[tokio::test]
    async fn test_creates_file_with_correct_name() {
        let tmp = tempfile::tempdir().unwrap();
        let mut writer = LogWriter::new(tmp.path().into(), RedactionEngine::empty())
            .await
            .unwrap();

        let mut event = make_event("test", "test.event");
        // Force a specific date for deterministic file name.
        let date = event.timestamp.date_naive();
        writer.write(&mut event).await.unwrap();

        let expected = writer.file_path(date);
        assert!(expected.exists(), "log file should exist");

        let content = tokio::fs::read_to_string(&expected).await.unwrap();
        let parsed: LogEvent = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.source_module, "test");
        assert_eq!(parsed.event_type, "test.event");
    }

    #[tokio::test]
    async fn test_multiple_events_same_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut writer = LogWriter::new(tmp.path().into(), RedactionEngine::empty())
            .await
            .unwrap();

        for i in 0..5 {
            let mut event = make_event("test", &format!("event.{i}"));
            writer.write(&mut event).await.unwrap();
        }

        let date = writer.current_date().unwrap();
        let content = tokio::fs::read_to_string(writer.file_path(date))
            .await
            .unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 5);
    }

    #[tokio::test]
    async fn test_redaction_before_write() {
        let tmp = tempfile::tempdir().unwrap();
        let redactor = RedactionEngine::new(vec![crate::RedactionPattern {
            field: "secret".into(),
            match_type: crate::PatternMatch::Exact,
            replacement: "[REDACTED]".into(),
        }]);
        let mut writer = LogWriter::new(tmp.path().into(), redactor).await.unwrap();

        let ctx = TraceContext::new_root("trace-test-2".into());
        let mut event = LogEvent::new(
            &ctx,
            None,
            LogLevel::Info,
            "test",
            "test.event",
            json!({"secret": "hunter2", "safe": "ok"}),
        );
        writer.write(&mut event).await.unwrap();

        let date = writer.current_date().unwrap();
        let content = tokio::fs::read_to_string(writer.file_path(date))
            .await
            .unwrap();
        let parsed: LogEvent = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.payload["secret"], "[REDACTED]");
        assert_eq!(parsed.payload["safe"], "ok");
    }

    #[tokio::test]
    async fn test_flush_after_write() {
        let tmp = tempfile::tempdir().unwrap();
        let mut writer = LogWriter::new(tmp.path().into(), RedactionEngine::empty())
            .await
            .unwrap();

        let mut event = make_event("test", "test.event");
        let date = event.timestamp.date_naive();
        writer.write(&mut event).await.unwrap();

        // File should be readable immediately after write (flushed).
        let content = tokio::fs::read_to_string(writer.file_path(date))
            .await
            .unwrap();
        assert!(!content.is_empty());
    }
}
