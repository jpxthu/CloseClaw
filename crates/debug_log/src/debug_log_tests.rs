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
