//! Unit tests for `debug_log_emitter` (Step 1.2 + 1.5 refactor).
//!
//! Covers:
//! 1. emit_debug_event with parent: child span's parent_span_id matches parent's span_id
//! 2. emit_debug_event without parent: root span has empty parent_span_id (backward compat)
//! 3. emit_debug_event: None debug_log is no-op
//! 4. emit_debug_event: empty trace_id is no-op
//! 5. Chained child spans: grandchild's parent_span_id matches child's span_id

use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel, TraceContext};
use tempfile::TempDir;

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn make_debug_log(temp_dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 1,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.expect("DebugLog::new failed")
}

async fn read_events_from_dir(dir: &std::path::Path) -> Vec<closeclaw_debug_log::LogEvent> {
    let mut events = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let content = tokio::fs::read_to_string(&path).await.unwrap();
            for line in content.lines() {
                if !line.trim().is_empty() {
                    if let Ok(event) = closeclaw_debug_log::LogEvent::from_jsonl(line) {
                        events.push(event);
                    }
                }
            }
        }
    }
    events
}

async fn read_events_with_timeout(dir: &std::path::Path) -> Vec<closeclaw_debug_log::LogEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let events = read_events_from_dir(dir).await;
        if !events.is_empty() || tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// emit_debug_event with parent creates a child span whose parent_span_id
/// matches the parent's span_id.
#[tokio::test]
async fn test_child_event_parent_span_id_matches() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;

    let parent = TraceContext::new_root("trace-child-test".to_string());
    let parent_span_id = parent.span_id.clone();

    crate::debug_log_emitter::emit_debug_event(crate::debug_log_emitter::EmitEventParams {
        ctx: crate::debug_log_emitter::DebugLogContext::new(
            Some(&debug_log),
            "trace-child-test",
            Some("sess-child-1"),
        ),
        level: LogLevel::Info,
        source_module: "gateway",
        event_type: "llm.call.start",
        payload: serde_json::json!({"model": "gpt-4o"}),
        parent: Some(&parent),
    });

    let events = read_events_with_timeout(temp_dir.path()).await;
    let child_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "llm.call.start")
        .collect();
    assert_eq!(child_events.len(), 1, "expected exactly one child event");
    let evt = &child_events[0];
    assert_eq!(evt.trace_id, "trace-child-test");
    assert_eq!(evt.parent_span_id, parent_span_id);
    assert_ne!(evt.span_id, parent_span_id, "child span_id must differ");
    assert_eq!(evt.source_module, "gateway");
    assert_eq!(evt.level, LogLevel::Info);
    assert_eq!(evt.session_key.as_deref(), Some("sess-child-1"));
    assert_eq!(evt.payload["model"].as_str().unwrap(), "gpt-4o");
}

/// emit_debug_event without parent creates a root span with empty
/// parent_span_id (backward compatibility).
#[tokio::test]
async fn test_root_event_empty_parent_span_id() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;

    crate::debug_log_emitter::emit_debug_event(crate::debug_log_emitter::EmitEventParams {
        ctx: crate::debug_log_emitter::DebugLogContext::new(
            Some(&debug_log),
            "trace-root-test",
            Some("sess-root-1"),
        ),
        level: LogLevel::Info,
        source_module: "gateway",
        event_type: "message.arrived",
        payload: serde_json::json!({"channel": "feishu"}),
        parent: None,
    });

    let events = read_events_with_timeout(temp_dir.path()).await;
    let root_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "message.arrived")
        .collect();
    assert_eq!(root_events.len(), 1, "expected exactly one root event");
    let evt = &root_events[0];
    assert_eq!(evt.trace_id, "trace-root-test");
    assert!(
        evt.parent_span_id.is_empty(),
        "root span must have empty parent_span_id"
    );
    assert!(!evt.span_id.is_empty(), "root span must have a span_id");
}

/// emit_debug_event with None debug_log is a no-op (no panic).
#[tokio::test]
async fn test_event_none_debug_log_no_op() {
    crate::debug_log_emitter::emit_debug_event(crate::debug_log_emitter::EmitEventParams {
        ctx: crate::debug_log_emitter::DebugLogContext::new(None, "trace-noop", Some("sess-noop")),
        level: LogLevel::Info,
        source_module: "gateway",
        event_type: "llm.call.start",
        payload: serde_json::json!({}),
        parent: None,
    });
    // No panic = success.
}

/// emit_debug_event with empty trace_id is a no-op (no panic).
#[tokio::test]
async fn test_event_empty_trace_id_no_op() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;

    crate::debug_log_emitter::emit_debug_event(crate::debug_log_emitter::EmitEventParams {
        ctx: crate::debug_log_emitter::DebugLogContext::new(
            Some(&debug_log),
            "",
            Some("sess-empty"),
        ),
        level: LogLevel::Info,
        source_module: "gateway",
        event_type: "test.event",
        payload: serde_json::json!({}),
        parent: None,
    });

    let events = read_events_with_timeout(temp_dir.path()).await;
    assert!(events.is_empty(), "empty trace_id should produce no events");
}

/// emit_debug_event with parent uses parent's trace_id.
#[tokio::test]
async fn test_child_event_inherits_parent_trace_id() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;

    let parent = TraceContext::new_root("trace-inherit".to_string());

    crate::debug_log_emitter::emit_debug_event(crate::debug_log_emitter::EmitEventParams {
        ctx: crate::debug_log_emitter::DebugLogContext::new(
            Some(&debug_log),
            "trace-inherit",
            None,
        ),
        level: LogLevel::Trace,
        source_module: "tools",
        event_type: "tool.execute",
        payload: serde_json::json!({"tool": "web_search"}),
        parent: Some(&parent),
    });

    let events = read_events_with_timeout(temp_dir.path()).await;
    let tool_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "tool.execute")
        .collect();
    assert_eq!(tool_events.len(), 1, "expected exactly one tool event");
    let evt = &tool_events[0];
    assert_eq!(evt.trace_id, "trace-inherit");
    assert_eq!(evt.parent_span_id, parent.span_id);
    assert!(evt.session_key.is_none(), "no session_key should be None");
}

/// Chained child spans: grandchild's parent_span_id matches child's span_id.
#[tokio::test]
async fn test_chained_child_spans() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;

    let root = TraceContext::new_root("trace-chain".to_string());
    let child = root.child();
    let child_span_id = child.span_id.clone();

    crate::debug_log_emitter::emit_debug_event(crate::debug_log_emitter::EmitEventParams {
        ctx: crate::debug_log_emitter::DebugLogContext::new(
            Some(&debug_log),
            "trace-chain",
            Some("sess-chain"),
        ),
        level: LogLevel::Info,
        source_module: "gateway",
        event_type: "tool.execute",
        payload: serde_json::json!({"depth": 2}),
        parent: Some(&child),
    });

    let events = read_events_with_timeout(temp_dir.path()).await;
    let tool_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "tool.execute")
        .collect();
    assert_eq!(tool_events.len(), 1);
    let evt = &tool_events[0];
    assert_eq!(evt.trace_id, "trace-chain");
    assert_eq!(
        evt.parent_span_id, child_span_id,
        "grandchild must point to child span"
    );
}
