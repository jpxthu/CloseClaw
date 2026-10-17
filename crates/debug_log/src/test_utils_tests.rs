//! Unit tests for the shared debug-log test helpers (feature `test-util`).

use super::*;
use crate::TraceContext;

fn make_event(trace_id: &str, event_type: &str, level: LogLevel) -> LogEvent {
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
async fn test_write_then_read_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let log = make_debug_log(&tmp).await;
    log.log(make_event("trace-rt", "test.roundtrip", LogLevel::Info))
        .await;

    let events = read_events(tmp.path()).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].trace_id, "trace-rt");
    assert_eq!(events[0].event_type, "test.roundtrip");
}

#[tokio::test]
async fn test_read_events_skips_blank_malformed_and_non_jsonl() {
    let tmp = TempDir::new().unwrap();
    let good_a = make_event("t-a", "type.a", LogLevel::Info);
    let good_b = make_event("t-b", "type.b", LogLevel::Info);
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
async fn test_filter_combinators_select_by_type_and_trace() {
    let events = vec![
        make_event("t1", "type.a", LogLevel::Info),
        make_event("t1", "type.b", LogLevel::Info),
        make_event("t2", "type.a", LogLevel::Info),
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
async fn test_level_baseline_difference_between_make_and_at_level() {
    let tmp_trace = TempDir::new().unwrap();
    let log = make_debug_log(&tmp_trace).await;
    assert_eq!(log.min_level(), LogLevel::Trace);
    log.log(make_event("t", "type.trace", LogLevel::Trace))
        .await;
    assert_eq!(read_events(tmp_trace.path()).await.len(), 1);

    let tmp_info = TempDir::new().unwrap();
    let log = make_debug_log_at_level(&tmp_info, LogLevel::Info).await;
    assert_eq!(log.min_level(), LogLevel::Info);
    log.log(make_event("t", "type.trace", LogLevel::Trace))
        .await;
    log.log(make_event("t", "type.debug", LogLevel::Debug))
        .await;
    assert!(
        read_events(tmp_info.path()).await.is_empty(),
        "below-baseline events must be dropped"
    );
    log.log(make_event("t", "type.info", LogLevel::Info)).await;
    let events = read_events(tmp_info.path()).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "type.info");
}

#[tokio::test]
async fn test_poll_returns_immediately_when_events_already_exist() {
    let tmp = TempDir::new().unwrap();
    let log = make_debug_log(&tmp).await;
    log.log(make_event("t", "type.ready", LogLevel::Info)).await;

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
async fn test_poll_captures_event_written_after_delay() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    let line = make_event("t-late", "type.late", LogLevel::Info)
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
async fn test_poll_short_deadline_timeout_returns_last_snapshot() {
    let tmp = TempDir::new().unwrap();

    let start = std::time::Instant::now();
    let events =
        poll_events_with_deadline(tmp.path(), |e| !e.is_empty(), Duration::from_millis(150)).await;
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
async fn test_poll_predicate_ignores_interference_until_target_event() {
    let tmp = TempDir::new().unwrap();
    let log = make_debug_log(&tmp).await;
    log.log(make_event("t", "type.noise", LogLevel::Info)).await;
    let dir = tmp.path().to_path_buf();
    let line = make_event("t", "type.target", LogLevel::Info)
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
async fn test_wait_for_event_type_returns_filtered_subset_only() {
    let tmp = TempDir::new().unwrap();
    let log = make_debug_log(&tmp).await;
    log.log(make_event("t", "type.hit", LogLevel::Info)).await;
    log.log(make_event("t", "type.miss", LogLevel::Info)).await;
    log.log(make_event("t", "type.hit", LogLevel::Info)).await;

    let hits = wait_for_event_type(tmp.path(), "type.hit").await;
    assert_eq!(hits.len(), 2, "only the requested event type is returned");
    assert!(hits.iter().all(|e| e.event_type == "type.hit"));
}
