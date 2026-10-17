//! Shared debug-log test helpers for the gateway crate.
//!
//! Consumed via `crate::debug_log_test_utils::...` from test modules.

use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogEvent, LogLevel};
use tempfile::TempDir;

/// Create a [`DebugLog`] writing to a temp directory with Trace-level logging.
pub async fn make_debug_log(temp_dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 7,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.expect("DebugLog::new failed")
}

/// Read all [`LogEvent`] entries from JSONL files in `dir`.
pub async fn read_events(dir: &std::path::Path) -> Vec<LogEvent> {
    let mut events = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    for line in content.lines() {
                        if !line.trim().is_empty() {
                            if let Ok(event) = LogEvent::from_jsonl(line) {
                                events.push(event);
                            }
                        }
                    }
                }
            }
        }
    }
    events
}

/// Poll [`read_events`] until events appear, up to 3 seconds.
pub async fn read_events_with_timeout(dir: &std::path::Path) -> Vec<LogEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let events = read_events(dir).await;
        if !events.is_empty() || tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Filter events by event type.
pub fn filter_events_by_type<'a>(events: &'a [LogEvent], event_type: &str) -> Vec<&'a LogEvent> {
    events
        .iter()
        .filter(|e| e.event_type == event_type)
        .collect()
}

/// Filter events by event type and trace_id.
pub fn filter_events_by_type_and_trace<'a>(
    events: &'a [LogEvent],
    event_type: &str,
    trace_id: &str,
) -> Vec<&'a LogEvent> {
    events
        .iter()
        .filter(|e| e.event_type == event_type && e.trace_id == trace_id)
        .collect()
}
