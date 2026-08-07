use tokio::sync::Mutex;
use tracing::{error, warn};

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
                error!(error = %e, "failed to write debug log event");
                return;
            }

            // Step 4: Check for day rotation → trigger retention cleanup
            let new_date = writer.current_date();
            if prev_date != new_date && new_date.is_some() {
                let retention = self.inner.retention.clone();
                tokio::spawn(async move {
                    match tokio::task::spawn_blocking(move || run_retention_cleanup(retention))
                        .await
                    {
                        Ok(Err(e)) => {
                            warn!(error = %e, "failed to cleanup expired log files");
                        }
                        Ok(Ok(_)) => {}
                        Err(e) => {
                            warn!(error = %e, "retention cleanup task panicked");
                        }
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

/// Run the retention cleanup (blocking I/O).
///
/// `LogRetention::cleanup_expired` uses synchronous filesystem I/O, so it
/// must be called from `tokio::task::spawn_blocking` to avoid blocking the
/// async runtime.
fn run_retention_cleanup(retention: LogRetention) -> Result<usize, crate::LogRetentionError> {
    retention.cleanup_expired()
}

/// Errors that can occur when creating a `DebugLog`.
#[derive(Debug, thiserror::Error)]
pub enum DebugLogError {
    #[error("failed to initialize log writer: {0}")]
    WriterInit(#[source] LogWriterError),
}

#[cfg(test)]
#[path = "debug_log_tests.rs"]
mod debug_log_tests;
