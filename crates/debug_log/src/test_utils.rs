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

/// Poll [`read_events`] until `predicate` passes, with the unified
/// default deadline (5s) and interval (20ms).
pub async fn poll_events<F>(dir: &Path, predicate: F) -> Vec<LogEvent>
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
    poll_events(dir, |events| !events.is_empty()).await
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
mod tests {
    use super::*;
    use crate::TraceContext;

    fn test_event(trace_id: &str, event_type: &str, level: LogLevel) -> LogEvent {
        let ctx = TraceContext::new_root(trace_id.to_string());
        LogEvent::new(
            &ctx,
            None,
            level,
            "test_utils",
            event_type,
            serde_json::json!({}),
        )
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let log = make_debug_log(&tmp).await;
        log.log(test_event("trace-rt", "test.roundtrip", LogLevel::Info))
            .await;

        let events = read_events(tmp.path()).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trace_id, "trace-rt");
        assert_eq!(events[0].event_type, "test.roundtrip");
    }

    #[tokio::test]
    async fn read_events_skips_blank_malformed_and_non_jsonl() {
        let tmp = TempDir::new().unwrap();
        let good_a = test_event("t-a", "type.a", LogLevel::Info);
        let good_b = test_event("t-b", "type.b", LogLevel::Info);
        let content = format!(
            "{}\n\n   \nnot-json\n{{\"broken\": 1}}\n{}\n",
            good_a.to_jsonl().unwrap(),
            good_b.to_jsonl().unwrap()
        );
        tokio::fs::write(tmp.path().join("debug-manual.jsonl"), content)
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("notes.txt"), "ignored\n")
            .await
            .unwrap();

        let events = read_events(tmp.path()).await;
        assert_eq!(events.len(), 2, "blank/malformed/non-jsonl must be skipped");
        assert_eq!(events[0].trace_id, "t-a");
        assert_eq!(events[1].trace_id, "t-b");
    }

    #[tokio::test]
    async fn filter_combinators_select_by_type_and_trace() {
        let events = vec![
            test_event("t1", "type.a", LogLevel::Info),
            test_event("t1", "type.b", LogLevel::Info),
            test_event("t2", "type.a", LogLevel::Info),
        ];

        let by_type = filter_events_by_type(&events, "type.a");
        assert_eq!(by_type.len(), 2);
        assert!(by_type.iter().all(|e| e.event_type == "type.a"));

        let by_type_and_trace = filter_events_by_type_and_trace(&events, "type.a", "t1");
        assert_eq!(by_type_and_trace.len(), 1);
        assert_eq!(by_type_and_trace[0].trace_id, "t1");

        assert!(filter_events_by_type(&events, "type.missing").is_empty());
    }

    #[tokio::test]
    async fn level_baseline_difference_between_make_and_at_level() {
        let tmp_trace = TempDir::new().unwrap();
        let log = make_debug_log(&tmp_trace).await;
        assert_eq!(log.min_level(), LogLevel::Trace);
        log.log(test_event("t", "type.trace", LogLevel::Trace))
            .await;
        assert_eq!(read_events(tmp_trace.path()).await.len(), 1);

        let tmp_info = TempDir::new().unwrap();
        let log = make_debug_log_at_level(&tmp_info, LogLevel::Info).await;
        assert_eq!(log.min_level(), LogLevel::Info);
        log.log(test_event("t", "type.trace", LogLevel::Trace))
            .await;
        log.log(test_event("t", "type.debug", LogLevel::Debug))
            .await;
        assert!(
            read_events(tmp_info.path()).await.is_empty(),
            "below-baseline events must be dropped"
        );
        log.log(test_event("t", "type.info", LogLevel::Info)).await;
        let events = read_events(tmp_info.path()).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "type.info");
    }

    #[tokio::test]
    async fn poll_returns_immediately_when_events_already_exist() {
        let tmp = TempDir::new().unwrap();
        let log = make_debug_log(&tmp).await;
        log.log(test_event("t", "type.ready", LogLevel::Info)).await;

        let start = std::time::Instant::now();
        let events =
            poll_events_with_deadline(tmp.path(), |events| !events.is_empty(), POLL_DEADLINE).await;
        assert_eq!(events.len(), 1);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "existing events must satisfy the predicate on the first poll"
        );
    }

    #[tokio::test]
    async fn poll_captures_event_written_after_delay() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();
        let line = test_event("t-late", "type.late", LogLevel::Info)
            .to_jsonl()
            .unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            tokio::fs::write(dir.join("late.jsonl"), format!("{line}\n"))
                .await
                .unwrap();
        });

        let events =
            poll_events_with_deadline(tmp.path(), |e| !e.is_empty(), Duration::from_secs(1)).await;
        assert_eq!(
            events.len(),
            1,
            "event written within the window must be captured"
        );
        assert_eq!(events[0].event_type, "type.late");
    }

    #[tokio::test]
    async fn poll_short_deadline_timeout_returns_last_snapshot() {
        let tmp = TempDir::new().unwrap();

        let start = std::time::Instant::now();
        let events =
            poll_events_with_deadline(tmp.path(), |e| !e.is_empty(), Duration::from_millis(150))
                .await;
        assert!(
            events.is_empty(),
            "timeout must return the last (empty) snapshot"
        );
        assert!(start.elapsed() >= Duration::from_millis(150));
        assert!(
            start.elapsed() < Duration::from_millis(900),
            "short deadline must not wait out the full 5s default"
        );
    }

    #[tokio::test]
    async fn poll_predicate_ignores_interference_until_target_event() {
        let tmp = TempDir::new().unwrap();
        let log = make_debug_log(&tmp).await;
        log.log(test_event("t", "type.noise", LogLevel::Info)).await;
        let dir = tmp.path().to_path_buf();
        let line = test_event("t", "type.target", LogLevel::Info)
            .to_jsonl()
            .unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            tokio::fs::write(dir.join("target.jsonl"), format!("{line}\n"))
                .await
                .unwrap();
        });

        let start = std::time::Instant::now();
        let events = poll_events_with_deadline(
            tmp.path(),
            |e| e.iter().any(|e| e.event_type == "type.target"),
            Duration::from_secs(1),
        )
        .await;
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(100),
            "interference events must not trigger the stop condition"
        );
        assert!(
            elapsed < Duration::from_millis(900),
            "target event must stop the poll before the deadline"
        );
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|e| e.event_type == "type.target"));
    }

    #[tokio::test]
    async fn wait_for_event_type_returns_filtered_subset_only() {
        let tmp = TempDir::new().unwrap();
        let log = make_debug_log(&tmp).await;
        log.log(test_event("t", "type.hit", LogLevel::Info)).await;
        log.log(test_event("t", "type.miss", LogLevel::Info)).await;
        log.log(test_event("t", "type.hit", LogLevel::Info)).await;

        let hits = wait_for_event_type(tmp.path(), "type.hit").await;
        assert_eq!(hits.len(), 2, "only the requested event type is returned");
        assert!(hits.iter().all(|e| e.event_type == "type.hit"));
    }
}
