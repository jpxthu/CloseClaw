use super::*;
use crate::TraceContext;
use serde_json::json;

fn make_event(session_key: Option<String>) -> LogEvent {
    let ctx = TraceContext::new_root("trace-test".into());
    LogEvent::new(
        &ctx,
        session_key,
        LogLevel::Info,
        "test_module",
        "test.event",
        json!({"key": "value", "count": 42}),
    )
}

#[test]
fn test_to_jsonl_is_single_line() {
    let event = make_event(None);
    let jsonl = event.to_jsonl().unwrap();
    assert!(!jsonl.contains('\n'), "JSONL should be a single line");
    assert!(
        !jsonl.contains('\r'),
        "JSONL should not contain carriage return"
    );
}

#[test]
fn test_to_jsonl_is_valid_json() {
    let event = make_event(None);
    let jsonl = event.to_jsonl().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&jsonl).unwrap();
    assert!(parsed.is_object());
}

#[test]
fn test_jsonl_roundtrip() {
    let event = make_event(None);
    let jsonl = event.to_jsonl().unwrap();
    let restored: LogEvent = LogEvent::from_jsonl(&jsonl).unwrap();
    assert_eq!(restored.trace_id, event.trace_id);
    assert_eq!(restored.span_id, event.span_id);
    assert_eq!(restored.parent_span_id, event.parent_span_id);
    assert_eq!(restored.level, event.level);
    assert_eq!(restored.source_module, event.source_module);
    assert_eq!(restored.event_type, event.event_type);
    assert_eq!(restored.payload, event.payload);
}

#[test]
fn test_session_key_present() {
    let event = make_event(Some("sess-123".into()));
    let jsonl = event.to_jsonl().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&jsonl).unwrap();
    assert_eq!(parsed["session_key"], "sess-123");
}

#[test]
fn test_session_key_absent_when_none() {
    let event = make_event(None);
    let jsonl = event.to_jsonl().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&jsonl).unwrap();
    assert!(
        parsed.get("session_key").is_none(),
        "session_key should be omitted when None"
    );
}

#[test]
fn test_all_fields_serialized() {
    let event = make_event(Some("sess-456".into()));
    let jsonl = event.to_jsonl().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&jsonl).unwrap();
    assert!(parsed.get("trace_id").is_some());
    assert!(parsed.get("span_id").is_some());
    assert!(parsed.get("parent_span_id").is_some());
    assert!(parsed.get("timestamp").is_some());
    assert!(parsed.get("level").is_some());
    assert!(parsed.get("source_module").is_some());
    assert!(parsed.get("event_type").is_some());
    assert!(parsed.get("payload").is_some());
    assert!(parsed.get("session_key").is_some());
}

#[test]
fn test_jsonl_roundtrip_with_session_key() {
    let event = make_event(Some("sess-789".into()));
    let jsonl = event.to_jsonl().unwrap();
    let restored: LogEvent = LogEvent::from_jsonl(&jsonl).unwrap();
    assert_eq!(restored.session_key, Some("sess-789".to_string()));
}

#[test]
fn test_jsonl_roundtrip_without_session_key() {
    let event = make_event(None);
    let jsonl = event.to_jsonl().unwrap();
    let restored: LogEvent = LogEvent::from_jsonl(&jsonl).unwrap();
    assert_eq!(restored.session_key, None);
}

#[test]
fn test_payload_preserved_through_jsonl() {
    let ctx = TraceContext::new_root("trace-1".into());
    let event = LogEvent::new(
        &ctx,
        None,
        LogLevel::Debug,
        "mod",
        "event",
        json!({"nested": {"a": [1, 2, 3]}, "flag": true}),
    );
    let jsonl = event.to_jsonl().unwrap();
    let restored: LogEvent = LogEvent::from_jsonl(&jsonl).unwrap();
    assert_eq!(restored.payload["nested"]["a"], json!([1, 2, 3]));
    assert_eq!(restored.payload["flag"], true);
}
