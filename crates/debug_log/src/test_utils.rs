//! Shared debug-log test helpers for the workspace.
//!
//! Gated behind `any(test, feature = "test-util")`: external consumers
//! wire it via a dev-dependency with the `test-util` feature
//! (`closeclaw-debug-log = { path = ..., features = ["test-util"] }`),
//! while in-crate tests reach it under `cfg(test)` without the feature
//! (a crate cannot enable its own feature). It never enters the default
//! build or the published API surface.

use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;

use crate::{DebugLog, DebugLogConfig, LogEvent, LogLevel, RedactionPattern};

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
    let config = make_config_at_level(temp_dir, min_level, vec![]);
    DebugLog::new(config).await.expect("DebugLog::new failed")
}

/// Build a [`DebugLogConfig`] pointing at `temp_dir` with a custom
/// minimum level and redaction patterns (Trace-baseline retention: 7
/// days). Config-level building block for in-crate tests that need a
/// non-baseline config (e.g. redaction behavior tests layering their
/// own patterns on top); feature consumers use the ready-made
/// [`make_debug_log`] / [`make_debug_log_at_level`].
pub(crate) fn make_config_at_level(
    temp_dir: &TempDir,
    min_level: LogLevel,
    redaction_patterns: Vec<RedactionPattern>,
) -> DebugLogConfig {
    DebugLogConfig {
        min_level,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 7,
        redaction_patterns,
    }
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

/// Count non-blank JSONL event lines across all `*.jsonl` files in `dir`.
///
/// Any fs read failure (`read_dir` / `next_entry` / `read_to_string`)
/// panics: the directory is created by `DebugLog::new`, so a read error
/// is an abnormal state and must not be silently treated as "zero
/// output" (which would make the absence assertions vacuously pass).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn count_events(dir: &Path) -> usize {
    let mut count = 0;
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .expect("read_dir on the log dir failed; treating it as zero output is a vacuous pass");
    while let Some(entry) = entries
        .next_entry()
        .await
        .expect("read_dir iteration failed; treating it as zero output is a vacuous pass")
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .expect("reading a *.jsonl event file failed; this is not zero output");
        count += content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
    }
    count
}

/// Poll the event count until it is non-zero or `timeout` expires, then
/// return the last observed count. Used both for absence assertions
/// (deadline exhausted at zero) and the positive control (non-zero).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn poll_event_count(dir: &Path, timeout: Duration) -> usize {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let count = count_events(dir).await;
        if count > 0 || tokio::time::Instant::now() >= deadline {
            return count;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
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
