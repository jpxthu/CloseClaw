use super::*;
use crate::{LogLevel, PatternMatch, RedactionPattern, TraceContext};

/// Helper: create a test LogEvent with default values.
fn make_event(level: LogLevel, trace_id: &str) -> crate::LogEvent {
    let ctx = TraceContext {
        trace_id: trace_id.to_string(),
        span_id: "span-1".to_string(),
        parent_span_id: String::new(),
    };
    crate::LogEvent::new(
        &ctx,
        Some("session-1".to_string()),
        level,
        "test_module",
        "test.event",
        serde_json::json!({"key": "value"}),
    )
}

/// Helper: create a JSONL file with given lines in the directory.
async fn write_jsonl(dir: &std::path::Path, filename: &str, lines: &[&str]) {
    let path = dir.join(filename);
    let content: String = lines.iter().map(|l| format!("{l}\n")).collect();
    tokio::fs::write(&path, content).await.unwrap();
}

#[tokio::test]
async fn test_read_events_from_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let event1 = make_event(LogLevel::Info, "trace-1");
    let event2 = make_event(LogLevel::Warn, "trace-2");
    let line1 = event1.to_jsonl().unwrap();
    let line2 = event2.to_jsonl().unwrap();

    write_jsonl(tmp.path(), "debug-2025-01-01.jsonl", &[&line1, &line2]).await;

    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].trace_id.as_deref(), Some("trace-1"));
    assert_eq!(events[1].trace_id.as_deref(), Some("trace-2"));
}

#[tokio::test]
async fn test_invalid_json_lines_are_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let event = make_event(LogLevel::Info, "trace-1");
    let line = event.to_jsonl().unwrap();

    write_jsonl(
        tmp.path(),
        "debug-2025-01-01.jsonl",
        &[&line, "not valid json", &line],
    )
    .await;

    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 2);
}

#[tokio::test]
async fn test_empty_file_returns_no_events() {
    let tmp = tempfile::tempdir().unwrap();
    write_jsonl(tmp.path(), "debug-2025-01-01.jsonl", &[]).await;

    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_nonexistent_directory_returns_error() {
    let consumer = JsonlLogConsumer::new(
        "/nonexistent/path".into(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    // Directory doesn't exist → read_dir fails
    let result = consumer.read_events().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_level_filter_excludes_low_level_events() {
    let tmp = tempfile::tempdir().unwrap();
    let trace_event = make_event(LogLevel::Trace, "trace-1");
    let info_event = make_event(LogLevel::Info, "trace-2");
    let warn_event = make_event(LogLevel::Warn, "trace-3");

    write_jsonl(
        tmp.path(),
        "debug-2025-01-01.jsonl",
        &[
            &trace_event.to_jsonl().unwrap(),
            &info_event.to_jsonl().unwrap(),
            &warn_event.to_jsonl().unwrap(),
        ],
    )
    .await;

    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Info),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].level, LogLevel::Info);
    assert_eq!(events[1].level, LogLevel::Warn);
}

#[tokio::test]
async fn test_redaction_engine_applies_to_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = TraceContext {
        trace_id: "trace-1".to_string(),
        span_id: "span-1".to_string(),
        parent_span_id: String::new(),
    };
    let event = crate::LogEvent::new(
        &ctx,
        None,
        LogLevel::Info,
        "test",
        "test.event",
        serde_json::json!({"api_key": "secret123", "normal_field": "visible"}),
    );

    write_jsonl(
        tmp.path(),
        "debug-2025-01-01.jsonl",
        &[&event.to_jsonl().unwrap()],
    )
    .await;

    let engine = RedactionEngine::new(vec![RedactionPattern {
        field: "api_key".to_string(),
        match_type: PatternMatch::Exact,
        replacement: "[REDACTED]".to_string(),
    }]);
    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        engine,
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].payload["api_key"],
        serde_json::Value::String("[REDACTED]".to_string())
    );
    assert_eq!(
        events[0].payload["normal_field"],
        serde_json::Value::String("visible".to_string())
    );
}

#[tokio::test]
async fn test_read_events_from_single_file() {
    let tmp = tempfile::tempdir().unwrap();
    let event = make_event(LogLevel::Info, "trace-1");
    let path = tmp.path().join("debug-2025-01-01.jsonl");
    tokio::fs::write(&path, event.to_jsonl().unwrap())
        .await
        .unwrap();

    let consumer = JsonlLogConsumer::new(
        path,
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn test_single_line_file() {
    let tmp = tempfile::tempdir().unwrap();
    let event = make_event(LogLevel::Error, "trace-1");
    write_jsonl(
        tmp.path(),
        "debug-2025-01-01.jsonl",
        &[&event.to_jsonl().unwrap()],
    )
    .await;

    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].level, LogLevel::Error);
}

#[tokio::test]
async fn test_mixed_valid_invalid_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let event1 = make_event(LogLevel::Info, "trace-1");
    let event2 = make_event(LogLevel::Warn, "trace-2");

    write_jsonl(
        tmp.path(),
        "debug-2025-01-01.jsonl",
        &[
            &event1.to_jsonl().unwrap(),
            "",
            "bad json {",
            &event2.to_jsonl().unwrap(),
        ],
    )
    .await;

    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 2);
}

#[tokio::test]
async fn test_non_framework_files_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let event = make_event(LogLevel::Info, "trace-1");

    // Framework file
    write_jsonl(
        tmp.path(),
        "debug-2025-01-01.jsonl",
        &[&event.to_jsonl().unwrap()],
    )
    .await;
    // Non-framework file (wrong prefix)
    write_jsonl(
        tmp.path(),
        "other-2025-01-01.jsonl",
        &[&event.to_jsonl().unwrap()],
    )
    .await;
    // Non-framework file (wrong extension)
    tokio::fs::write(tmp.path().join("debug-2025-01-01.txt"), "not jsonl")
        .await
        .unwrap();

    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn test_normalized_event_preserves_all_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = TraceContext {
        trace_id: "trace-42".to_string(),
        span_id: "span-7".to_string(),
        parent_span_id: "parent-3".to_string(),
    };
    let event = crate::LogEvent::new(
        &ctx,
        Some("session-99".to_string()),
        LogLevel::Debug,
        "gateway",
        "message.arrived",
        serde_json::json!({"msg": "hello"}),
    );

    write_jsonl(
        tmp.path(),
        "debug-2025-01-01.jsonl",
        &[&event.to_jsonl().unwrap()],
    )
    .await;

    let consumer = JsonlLogConsumer::new(
        tmp.path().to_path_buf(),
        RedactionEngine::empty(),
        LevelFilter::new(LogLevel::Trace),
    );

    let events = consumer.read_events().await.unwrap();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.trace_id.as_deref(), Some("trace-42"));
    assert_eq!(e.span_id, "span-7");
    assert_eq!(e.parent_span_id, "parent-3");
    assert_eq!(e.session_key.as_deref(), Some("session-99"));
    assert_eq!(e.level, LogLevel::Debug);
    assert_eq!(e.source_module, "gateway");
    assert_eq!(e.event_type, "message.arrived");
}
