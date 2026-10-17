//! Shared debug-log test helpers for the workspace.
//!
//! Feature-gated behind `test-util` so the helpers never enter the
//! default build or the published API surface. Consumers wire it via a
//! dev-dependency:
//! `closeclaw-debug-log = { path = ..., features = ["test-util"] }`.

use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;

use crate::{DebugLog, DebugLogConfig, LogEvent, LogLevel};

/// Deadline for the convenience polling helpers — the widest of the
/// unified deadline families (avoids false timeouts on slow machines).
const POLL_DEADLINE: Duration = Duration::from_secs(5);

/// Poll interval for all polling helpers (the fastest unified value).
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Create a [`DebugLog`] writing to a temp directory with Trace-level
/// logging (mainstream baseline: Trace level, 7-day retention).
pub async fn make_debug_log(temp_dir: &TempDir) -> DebugLog {
    make_debug_log_at_level(temp_dir, LogLevel::Trace).await
}

/// Create a [`DebugLog`] writing to a temp directory at a custom
/// minimum level (for consumers needing a non-Trace baseline).
pub async fn make_debug_log_at_level(temp_dir: &TempDir, min_level: LogLevel) -> DebugLog {
    let config = DebugLogConfig {
        min_level,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 7,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.expect("DebugLog::new failed")
}

/// Read all [`LogEvent`] entries from the JSONL files in `dir` (async I/O).
///
/// Behavior: skips non-`.jsonl` files, skips blank lines, and silently
/// skips malformed lines (never panics).
pub async fn read_events(dir: &Path) -> Vec<LogEvent> {
    let mut events = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return events;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(content) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(event) = LogEvent::from_jsonl(line) {
                events.push(event);
            }
        }
    }
    events
}

/// Poll [`read_events`] until `predicate` passes, up to `timeout`.
///
/// Returns the last event snapshot — the one satisfying the predicate,
/// or the final snapshot observed when the deadline expires.
pub async fn poll_events_with_deadline<F>(
    dir: &Path,
    predicate: F,
    timeout: Duration,
) -> Vec<LogEvent>
where
    F: Fn(&[LogEvent]) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let events = read_events(dir).await;
        if predicate(&events) || tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Poll until any event appears, up to `timeout` (interval 20ms).
///
/// Thin wrapper over [`poll_events_with_deadline`] with the shared
/// non-empty stop condition — the negative-path-friendly variant of
/// [`read_events_with_timeout`].
pub async fn read_events_with_deadline(dir: &Path, timeout: Duration) -> Vec<LogEvent> {
    poll_events_with_deadline(dir, |events| !events.is_empty(), timeout).await
}

/// Poll [`read_events`] until `predicate` passes, with the unified
/// default deadline (5s) and interval (20ms).
async fn poll_events<F>(dir: &Path, predicate: F) -> Vec<LogEvent>
where
    F: Fn(&[LogEvent]) -> bool,
{
    poll_events_with_deadline(dir, predicate, POLL_DEADLINE).await
}

/// Wait until at least one event of `event_type` lands in the JSONL
/// files (deadline 5s / interval 20ms), then return the filtered subset.
///
/// Stop condition: "filtered result is non-empty" — the widened deadline
/// only extends failure-path waiting, never changes the stop semantics.
pub async fn wait_for_event_type(dir: &Path, event_type: &str) -> Vec<LogEvent> {
    let events = poll_events(dir, |events| {
        events.iter().any(|e| e.event_type == event_type)
    })
    .await;
    events
        .into_iter()
        .filter(|e| e.event_type == event_type)
        .collect()
}

/// Poll until any event appears (legacy convenience, gateway
/// semantics) with the unified default deadline/interval.
pub async fn read_events_with_timeout(dir: &Path) -> Vec<LogEvent> {
    read_events_with_deadline(dir, POLL_DEADLINE).await
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

#[cfg(test)]
#[path = "test_utils_tests.rs"]
mod tests;
