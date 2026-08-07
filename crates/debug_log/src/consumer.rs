use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::io::AsyncBufReadExt;

use crate::{LevelFilter, LogLevel, RedactionEngine};

/// A log event normalized for consumption by the operations agent.
///
/// Fields mirror [`LogEvent`](crate::LogEvent), but `trace_id` and
/// `session_key` are optional since module logs may lack framework
/// tracing metadata.
#[derive(Debug, Clone)]
pub struct NormalizedEvent {
    /// Message chain unique identifier (optional if absent in source).
    pub trace_id: Option<String>,
    /// Current span identifier.
    pub span_id: String,
    /// Parent span identifier (empty for root spans).
    pub parent_span_id: String,
    /// Optional session key for message chain correlation.
    pub session_key: Option<String>,
    /// Event timestamp in milliseconds (UTC).
    pub timestamp: i64,
    /// Event severity level.
    pub level: LogLevel,
    /// Source module that produced this event.
    pub source_module: String,
    /// Event type for categorization.
    pub event_type: String,
    /// Structured payload as arbitrary JSON.
    pub payload: serde_json::Value,
}

/// Trait for consuming and normalizing log events from external sources.
#[async_trait]
pub trait LogConsumer: Send + Sync {
    /// Read and normalize log events from the configured source.
    ///
    /// Returns a list of normalized events. Sources that fail to parse
    /// individual lines are skipped silently (logged at warn level).
    async fn read_events(&self) -> Result<Vec<NormalizedEvent>, ConsumerError>;
}

/// Errors that can occur during log consumption.
#[derive(Debug, thiserror::Error)]
pub enum ConsumerError {
    #[error("failed to read log directory: {0}")]
    ReadDir(#[source] std::io::Error),
    #[error("failed to open log file: {0}")]
    OpenFile(#[source] std::io::Error),
    #[error("failed to read log line: {0}")]
    ReadLine(#[source] std::io::Error),
}

/// Default [`LogConsumer`] implementation that reads JSONL files.
///
/// Reads all files matching `debug-*.jsonl` in the given directory (or a
/// single file if a file path is provided). Each line is parsed as a
/// JSON [`LogEvent`](crate::LogEvent), normalized into a
/// [`NormalizedEvent`], filtered by [`LevelFilter`], and redacted via
/// [`RedactionEngine`].
pub struct JsonlLogConsumer {
    source_path: PathBuf,
    redaction_engine: RedactionEngine,
    level_filter: LevelFilter,
}

impl JsonlLogConsumer {
    /// Create a new consumer.
    ///
    /// - `source_path`: path to a JSONL file or directory of JSONL files
    /// - `redaction_engine`: engine used to redact sensitive fields
    /// - `level_filter`: minimum log level to include in output
    pub fn new(
        source_path: PathBuf,
        redaction_engine: RedactionEngine,
        level_filter: LevelFilter,
    ) -> Self {
        Self {
            source_path,
            redaction_engine,
            level_filter,
        }
    }

    /// Collect all JSONL files from the source path.
    ///
    /// If the source is a single file, returns just that file.
    /// If the source is a directory, returns all `debug-*.jsonl` files.
    async fn collect_files(&self) -> Result<Vec<PathBuf>, ConsumerError> {
        if self.source_path.is_file() {
            return Ok(vec![self.source_path.clone()]);
        }

        let mut entries = tokio::fs::read_dir(&self.source_path)
            .await
            .map_err(ConsumerError::ReadDir)?;

        let mut files = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(ConsumerError::ReadDir)? {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if Self::is_framework_file(&path) {
                files.push(path);
            }
        }

        Ok(files)
    }

    /// Check if a path matches the `debug-*.jsonl` naming pattern.
    fn is_framework_file(path: &std::path::Path) -> bool {
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => return false,
        };
        let ext = match path.extension().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => return false,
        };
        ext == "jsonl" && stem.starts_with("debug-")
    }

    /// Read and parse a single JSONL file, normalizing each event.
    async fn read_file(&self, path: &Path) -> Result<Vec<NormalizedEvent>, ConsumerError> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(ConsumerError::OpenFile)?;
        let reader = tokio::io::BufReader::new(file);
        let mut lines = reader.lines();
        let mut events = Vec::new();

        while let Some(line) = lines.next_line().await.map_err(ConsumerError::ReadLine)? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<crate::LogEvent>(line) {
                Ok(event) => {
                    if !self.level_filter.should_log(&event.level) {
                        continue;
                    }
                    let mut normalized = NormalizedEvent::from_log_event(event);
                    self.redaction_engine.redact(&mut normalized.payload);
                    events.push(normalized);
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "skipping unparseable log line"
                    );
                }
            }
        }

        Ok(events)
    }
}

impl NormalizedEvent {
    /// Convert a [`LogEvent`](crate::LogEvent) into a `NormalizedEvent`.
    ///
    /// The `trace_id` from the original event is preserved as `Some`.
    pub fn from_log_event(event: crate::LogEvent) -> Self {
        Self {
            trace_id: Some(event.trace_id),
            span_id: event.span_id,
            parent_span_id: event.parent_span_id,
            session_key: event.session_key,
            timestamp: event.timestamp,
            level: event.level,
            source_module: event.source_module,
            event_type: event.event_type,
            payload: event.payload,
        }
    }
}

#[async_trait]
impl LogConsumer for JsonlLogConsumer {
    async fn read_events(&self) -> Result<Vec<NormalizedEvent>, ConsumerError> {
        let files = self.collect_files().await?;
        let mut all_events = Vec::new();

        for file in files {
            let events = self.read_file(&file).await?;
            all_events.extend(events);
        }

        Ok(all_events)
    }
}

#[cfg(test)]
#[path = "consumer_tests.rs"]
mod consumer_tests;
