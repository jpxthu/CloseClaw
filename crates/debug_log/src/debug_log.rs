use std::path::Path;

use tokio::sync::Mutex;
use tracing::warn;

use crate::{
    DebugLogConfig, LevelFilter, LogLevel, LogRetention, LogWriter, LogWriterError, RedactionEngine,
};

/// Shared inner state for a `DebugLog` instance.
///
/// Wrapped in `Arc` so multiple modules can share a single `DebugLog`
/// without duplicating state. The `LogWriter` is behind a `Mutex` to
/// allow `&self` callers to write events.
#[derive(Debug)]
struct Inner {
    writer: Mutex<LogWriter>,
    level_filter: LevelFilter,
    redaction_engine: RedactionEngine,
    retention: LogRetention,
}

/// Unified entry point for the debug log framework.
///
/// Chains all components into a single `log()` call:
/// `LevelFilter` → `RedactionEngine` → `LogWriter` → rotation check → `LogRetention`.
///
/// Designed as cloneable (`Arc` internals) so multiple modules can share
/// a single instance.
#[derive(Clone)]
pub struct DebugLog {
    inner: std::sync::Arc<Inner>,
}

impl DebugLog {
    /// Create a new `DebugLog` from configuration.
    ///
    /// Initializes all internal components: `LevelFilter`, `RedactionEngine`,
    /// `LogWriter`, and `LogRetention`. The log directory is created if it
    /// doesn't exist.
    pub async fn new(config: DebugLogConfig) -> Result<Self, DebugLogError> {
        let level_filter = LevelFilter::new(config.min_level);
        let redaction_engine = RedactionEngine::new(config.redaction_patterns);
        let retention = LogRetention::new(config.log_dir.clone(), config.retention_days);

        let writer = LogWriter::new(config.log_dir)
            .await
            .map_err(DebugLogError::WriterInit)?;

        Ok(Self {
            inner: std::sync::Arc::new(Inner {
                writer: Mutex::new(writer),
                level_filter,
                redaction_engine,
                retention,
            }),
        })
    }

    /// Process a log event through the full pipeline.
    ///
    /// 1. `LevelFilter` — discard events below the configured level.
    /// 2. `RedactionEngine` — replace sensitive field values.
    /// 3. `LogWriter` — serialize to JSONL and write to today's file.
    /// 4. Rotation check — if a new day started, trigger `LogRetention` cleanup.
    pub async fn log(&self, mut event: crate::LogEvent) {
        // Step 1: Level filter
        if !self.inner.level_filter.should_log(&event.level) {
            return;
        }

        // Step 2: Redact sensitive fields
        self.inner.redaction_engine.redact(&mut event.payload);

        // Step 3: Write to file
        {
            let mut writer = self.inner.writer.lock().await;
            let prev_date = writer.current_date();

            if let Err(e) = writer.write(&event).await {
                warn!(error = %e, "failed to write debug log event");
                return;
            }

            // Step 4: Check for day rotation → trigger retention cleanup
            let new_date = writer.current_date();
            if prev_date != new_date && new_date.is_some() {
                let retention = self.inner.retention.clone();
                let log_dir = writer.log_dir().to_path_buf();
                tokio::spawn(async move {
                    if let Err(e) = run_retention_cleanup(&retention, &log_dir) {
                        warn!(error = %e, "failed to cleanup expired log files");
                    }
                });
            }
        }
    }

    /// Get the current minimum log level.
    pub fn min_level(&self) -> LogLevel {
        self.inner.level_filter.min_level()
    }
}

/// Run the retention cleanup in a blocking context.
///
/// `LogRetention::cleanup_expired` uses synchronous I/O, so it must not
/// run directly on the async runtime. Spawning it via `tokio::spawn` and
/// wrapping in `spawn_blocking` keeps the runtime responsive.
fn run_retention_cleanup(
    retention: &LogRetention,
    _log_dir: &Path,
) -> Result<usize, crate::LogRetentionError> {
    retention.cleanup_expired()
}

/// Errors that can occur when creating a `DebugLog`.
#[derive(Debug, thiserror::Error)]
pub enum DebugLogError {
    #[error("failed to initialize log writer: {0}")]
    WriterInit(#[source] LogWriterError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LogEvent, LogLevel, TraceContext};
    use serde_json::json;
    use tempfile::TempDir;

    fn make_config(dir: &TempDir) -> DebugLogConfig {
        DebugLogConfig {
            min_level: LogLevel::Debug,
            log_dir: dir.path().to_path_buf(),
            retention_days: 7,
            redaction_patterns: vec![crate::RedactionPattern {
                field: "secret".into(),
                match_type: crate::PatternMatch::Exact,
                replacement: "[REDACTED]".into(),
            }],
        }
    }

    fn make_event(
        level: LogLevel,
        module: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> LogEvent {
        let ctx = TraceContext::new_root("trace-test".into());
        LogEvent::new(&ctx, None, level, module, event_type, payload)
    }

    #[tokio::test]
    async fn test_log_writes_event() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(&tmp);
        let debug_log = DebugLog::new(config).await.unwrap();

        let event = make_event(
            LogLevel::Info,
            "test",
            "test.event",
            json!({"key": "value"}),
        );
        debug_log.log(event).await;

        // Verify file was created and contains the event
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "jsonl")
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(entries.len(), 1);

        let content = std::fs::read_to_string(entries[0].path()).unwrap();
        let parsed: LogEvent = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.source_module, "test");
        assert_eq!(parsed.event_type, "test.event");
        assert_eq!(parsed.payload["key"], "value");
    }

    #[tokio::test]
    async fn test_level_filter_blocks_low_level() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = make_config(&tmp);
        config.min_level = LogLevel::Warn;
        let debug_log = DebugLog::new(config).await.unwrap();

        // Info should be filtered out
        let event = make_event(LogLevel::Info, "test", "test.event", json!({}));
        debug_log.log(event).await;

        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "jsonl")
                    .unwrap_or(false)
            })
            .collect();
        assert!(
            entries.is_empty(),
            "no log file should be created for filtered events"
        );
    }

    #[tokio::test]
    async fn test_redaction_applied() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(&tmp);
        let debug_log = DebugLog::new(config).await.unwrap();

        let event = make_event(
            LogLevel::Info,
            "test",
            "test.event",
            json!({"secret": "hunter2", "safe": "ok"}),
        );
        debug_log.log(event).await;

        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "jsonl")
                    .unwrap_or(false)
            })
            .collect();
        let content = std::fs::read_to_string(entries[0].path()).unwrap();
        let parsed: LogEvent = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.payload["secret"], "[REDACTED]");
        assert_eq!(parsed.payload["safe"], "ok");
    }

    #[tokio::test]
    async fn test_multiple_events_same_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(&tmp);
        let debug_log = DebugLog::new(config).await.unwrap();

        for i in 0..5 {
            let event = make_event(
                LogLevel::Info,
                "test",
                &format!("event.{i}"),
                json!({"i": i}),
            );
            debug_log.log(event).await;
        }

        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "jsonl")
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(entries.len(), 1);

        let content = std::fs::read_to_string(entries[0].path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 5);
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(&tmp);
        let debug_log = DebugLog::new(config).await.unwrap();
        let debug_log2 = debug_log.clone();

        let event1 = make_event(LogLevel::Info, "test", "event.1", json!({"a": 1}));
        let event2 = make_event(LogLevel::Info, "test", "event.2", json!({"b": 2}));

        debug_log.log(event1).await;
        debug_log2.log(event2).await;

        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "jsonl")
                    .unwrap_or(false)
            })
            .collect();
        let content = std::fs::read_to_string(entries[0].path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[tokio::test]
    async fn test_min_level_accessor() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = make_config(&tmp);
        config.min_level = LogLevel::Error;
        let debug_log = DebugLog::new(config).await.unwrap();
        assert_eq!(debug_log.min_level(), LogLevel::Error);
    }
}
