use super::*;
use crate::{LogLevel, TraceContext};
use chrono::DateTime;
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
    let mut writer = LogWriter::new(tmp.path().into()).await.unwrap();

    let event = make_event("test", "test.event");
    // Force a specific date for deterministic file name.
    let date = DateTime::from_timestamp_millis(event.timestamp)
        .unwrap()
        .date_naive();
    writer.write(&event).await.unwrap();

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
    let mut writer = LogWriter::new(tmp.path().into()).await.unwrap();

    for i in 0..5 {
        let event = make_event("test", &format!("event.{i}"));
        writer.write(&event).await.unwrap();
    }

    let date = writer.current_date().unwrap();
    let content = tokio::fs::read_to_string(writer.file_path(date))
        .await
        .unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 5);
}

#[tokio::test]
async fn test_flush_after_write() {
    let tmp = tempfile::tempdir().unwrap();
    let mut writer = LogWriter::new(tmp.path().into()).await.unwrap();

    let event = make_event("test", "test.event");
    let date = DateTime::from_timestamp_millis(event.timestamp)
        .unwrap()
        .date_naive();
    writer.write(&event).await.unwrap();

    // File should be readable immediately after write (flushed).
    let content = tokio::fs::read_to_string(writer.file_path(date))
        .await
        .unwrap();
    assert!(!content.is_empty());
}
