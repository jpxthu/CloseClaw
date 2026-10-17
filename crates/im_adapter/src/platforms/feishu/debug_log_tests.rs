//! Unit tests for FeishuPlugin debug_log framework integration (Step 1.4).
//!
//! Test dimensions:
//! 1. Normal path: debug_log set → parse_inbound/render/send write JSONL
//! 2. No debug_log path: None doesn't panic
//! 3. trace_id propagation: parse → last_metadata → render/send read same
//! 4. Timing accuracy: durations are non-negative
//! 5. Event structure completeness: JSONL has all required fields

use super::adapter::FeishuAdapter;
use super::make_adapter;
use super::FeishuPlugin;
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::IMPlugin;
use closeclaw_debug_log::test_utils::{make_debug_log, read_events, wait_for_event_type};
use closeclaw_debug_log::LogLevel;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Pre-populate last_metadata with a trace_id for tests that call
/// render/send without first calling parse_inbound.
async fn set_test_trace_id(adapter: &FeishuAdapter, trace_id: &str) {
    let mut meta = adapter.last_metadata.lock().await;
    meta.insert("trace_id".to_string(), trace_id.to_string());
}

fn make_text_payload(text: &str) -> Vec<u8> {
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_debug_log_test",
            "event_type": "im.message.receive_v1",
            "create_time": "1234567890",
            "token": "tok",
            "app_id": "test_app_id"
        },
        "event": {
            "sender": {
                "sender_id": { "open_id": "ou_sender" },
                "sender_type": "user"
            },
            "content": serde_json::json!({"text": text}).to_string(),
            "chat_id": "oc_chat",
            "message_type": "text"
        }
    });
    serde_json::to_vec(&payload).unwrap()
}

// ===========================================================================
// 1. Normal path: debug_log set → parse_inbound/render/send write JSONL
// ===========================================================================

/// parse_inbound with debug_log writes inbound.parse event to JSONL.
#[tokio::test]
async fn test_parse_inbound_writes_jsonl() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = make_text_payload("hello jsonl");
    let _ = plugin.parse_inbound(&payload).await.unwrap();

    let events = wait_for_event_type(temp_dir.path(), "inbound.parse").await;
    assert!(!events.is_empty(), "parse_inbound should write JSONL");
    let evt = &events[0];
    assert_eq!(evt.event_type, "inbound.parse");
    assert_eq!(evt.payload["platform"], "feishu");
    assert_eq!(evt.payload["message_type"], "text");
    assert!(
        evt.payload["parse_duration_ms"].is_number(),
        "parse_duration_ms should be a number"
    );
}

/// render with debug_log writes outbound.render event to JSONL.
/// Uses multiline text to trigger the card rendering path (bypasses early return).
#[tokio::test]
async fn test_render_writes_jsonl() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    set_test_trace_id(&adapter, "test-render-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    // Multiline text triggers card rendering path (bypasses early return).
    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let _ = plugin.render(&blocks, None);

    let events = wait_for_event_type(temp_dir.path(), "outbound.render").await;
    assert!(!events.is_empty(), "render should write JSONL");
    let evt = &events[0];
    assert_eq!(evt.event_type, "outbound.render");
    assert_eq!(evt.payload["platform"], "feishu");
    assert!(
        evt.payload["msg_type"].is_string(),
        "msg_type should be a string"
    );
    assert!(
        evt.payload["render_duration_ms"].is_number(),
        "render_duration_ms should be a number"
    );
}

/// send with debug_log writes outbound.send event to JSONL.
#[tokio::test]
async fn test_send_writes_jsonl() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    set_test_trace_id(&adapter, "test-send-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let output = closeclaw_common::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "send test"}}),
    };
    let result = plugin.send(&output, "ou_peer", None, None).await;
    assert!(result.is_ok());

    let events = wait_for_event_type(temp_dir.path(), "outbound.send").await;
    assert!(!events.is_empty(), "send should write JSONL");
    let evt = &events[0];
    assert_eq!(evt.event_type, "outbound.send");
    assert_eq!(evt.payload["platform"], "feishu");
    assert_eq!(evt.payload["peer_id"], "ou_peer");
    assert_eq!(evt.payload["msg_type"], "text");
    assert!(
        evt.payload["send_duration_ms"].is_number(),
        "send_duration_ms should be a number"
    );
    assert!(
        evt.payload["success"].is_boolean(),
        "success should be a boolean"
    );
}

// ===========================================================================
// 2. No debug_log path: None doesn't panic
// ===========================================================================

/// parse_inbound without debug_log works normally (no panic).
#[tokio::test]
async fn test_parse_inbound_without_debug_log_no_panic() {
    let adapter = Arc::new(make_adapter("unused-cli"));
    let plugin = FeishuPlugin::new(adapter);

    let payload = make_text_payload("no debug_log");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    let msg = result.expect("should return Some for valid payload");
    assert_eq!(msg.content, "no debug_log");
    assert_eq!(msg.platform, "feishu");
}

/// render without debug_log works normally (no panic).
#[tokio::test]
async fn test_render_without_debug_log_no_panic() {
    let adapter = Arc::new(make_adapter("unused-cli"));
    let plugin = FeishuPlugin::new(adapter);

    let blocks = vec![ContentBlock::Text("plain".into())];
    let output = plugin.render(&blocks, None);
    assert_eq!(output.msg_type, "text");
}

/// send without debug_log works normally (no panic).
#[tokio::test]
async fn test_send_without_debug_log_no_panic() {
    let adapter = Arc::new(make_adapter("unused-cli"));
    let plugin = FeishuPlugin::new(adapter);

    let output = closeclaw_common::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "test"}}),
    };
    let result = plugin.send(&output, "ou_peer", None, None).await;
    assert!(result.is_ok());
}

/// parse_inbound with empty text returns None without panic (no debug_log).
#[tokio::test]
async fn test_parse_inbound_empty_text_no_debug_log() {
    let adapter = Arc::new(make_adapter("unused-cli"));
    let plugin = FeishuPlugin::new(adapter);

    let payload = make_text_payload("");
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(result.is_none(), "empty text should be discarded");
}

// ===========================================================================
// 3. trace_id propagation
// ===========================================================================

/// parse_inbound generates trace_id in last_metadata; render reads it.
#[tokio::test]
async fn test_trace_id_propagation_render() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    // parse_inbound generates trace_id (async)
    let payload = make_text_payload("trace test");
    let _ = plugin.parse_inbound(&payload).await.unwrap();

    // Verify trace_id was stored
    let meta = plugin.last_parsed_metadata();
    let trace_id = meta
        .get("trace_id")
        .expect("trace_id should be in metadata");
    assert!(!trace_id.is_empty(), "trace_id should not be empty");

    // render reads trace_id from last_metadata
    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let _ = plugin.render(&blocks, None);

    let events = wait_for_event_type(temp_dir.path(), "outbound.render").await;
    assert!(!events.is_empty(), "render event should exist");
    let evt = &events[0];
    assert_eq!(
        evt.trace_id.as_str(),
        trace_id,
        "render trace_id should match parse_inbound trace_id"
    );
}

/// parse_inbound generates trace_id in last_metadata; send reads it.
#[tokio::test]
async fn test_trace_id_propagation_send() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    // parse_inbound generates trace_id
    let payload = make_text_payload("trace send");
    let _ = plugin.parse_inbound(&payload).await.unwrap();

    let meta = plugin.last_parsed_metadata();
    let trace_id = meta
        .get("trace_id")
        .expect("trace_id should be in metadata");

    // send should read the same trace_id
    let output = closeclaw_common::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "send trace"}}),
    };
    let _ = plugin.send(&output, "ou_peer", None, None).await;

    let events = wait_for_event_type(temp_dir.path(), "outbound.send").await;
    assert!(!events.is_empty(), "send event should exist");
    let evt = &events[0];
    assert_eq!(
        evt.trace_id.as_str(),
        trace_id,
        "send trace_id should match parse_inbound trace_id"
    );
}

// ===========================================================================
// 4. Timing accuracy: durations are non-negative
// ===========================================================================

/// parse_duration_ms is non-negative and reasonable.
#[tokio::test]
async fn test_parse_duration_non_negative() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = make_text_payload("timing parse");
    let _ = plugin.parse_inbound(&payload).await.unwrap();

    let events = wait_for_event_type(temp_dir.path(), "inbound.parse").await;
    assert!(!events.is_empty());
    let evt = &events[0];
    let ms = evt.payload["parse_duration_ms"]
        .as_u64()
        .expect("parse_duration_ms should be u64");
    assert!(
        ms <= 30_000,
        "parse_duration_ms should be reasonable, got {}",
        ms
    );
}

/// render_duration_ms is non-negative and reasonable.
/// Uses multiline text to trigger the card rendering path.
#[tokio::test]
async fn test_render_duration_non_negative() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    set_test_trace_id(&adapter, "test-render-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let _ = plugin.render(&blocks, None);

    let events = wait_for_event_type(temp_dir.path(), "outbound.render").await;
    assert!(!events.is_empty());
    let evt = &events[0];
    let ms = evt.payload["render_duration_ms"]
        .as_u64()
        .expect("render_duration_ms should be u64");
    assert!(
        ms <= 30_000,
        "render_duration_ms should be reasonable, got {}",
        ms
    );
}

/// send_duration_ms is non-negative and reasonable.
#[tokio::test]
async fn test_send_duration_non_negative() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    set_test_trace_id(&adapter, "test-send-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let output = closeclaw_common::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "timing send"}}),
    };
    let _ = plugin.send(&output, "ou_peer", None, None).await;

    let events = wait_for_event_type(temp_dir.path(), "outbound.send").await;
    assert!(!events.is_empty());
    let evt = &events[0];
    let ms = evt.payload["send_duration_ms"]
        .as_u64()
        .expect("send_duration_ms should be u64");
    assert!(
        ms <= 30_000,
        "send_duration_ms should be reasonable, got {}",
        ms
    );
}

// ===========================================================================
// 5. Event structure completeness
//
// Required fields present and well-formed (missing or mistyped fields
// fail LogEvent deserialization and get skipped by read_events,
// surfacing here as an empty/absent event).
// ===========================================================================

/// inbound.parse JSONL contains all required fields.
#[tokio::test]
async fn test_inbound_parse_event_structure() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = make_text_payload("structure test");
    let _ = plugin.parse_inbound(&payload).await.unwrap();

    let events = wait_for_event_type(temp_dir.path(), "inbound.parse").await;
    assert!(!events.is_empty(), "inbound.parse event should exist");
    let evt = &events[0];

    assert!(!evt.trace_id.is_empty(), "missing trace_id");
    assert!(!evt.span_id.is_empty(), "missing span_id");
    assert!(evt.timestamp > 0, "missing timestamp");
    assert_eq!(evt.level, LogLevel::Info, "missing level");
    assert_eq!(evt.source_module, "feishu", "missing source_module");
    assert_eq!(evt.event_type, "inbound.parse", "missing event_type");
    assert!(evt.payload.is_object(), "missing payload");
}

/// outbound.render JSONL contains all required fields.
/// Uses multiline text to trigger the card rendering path.
#[tokio::test]
async fn test_outbound_render_event_structure() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    set_test_trace_id(&adapter, "test-render-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let _ = plugin.render(&blocks, None);

    let events = wait_for_event_type(temp_dir.path(), "outbound.render").await;
    assert!(!events.is_empty(), "outbound.render event should exist");
    let evt = &events[0];

    assert!(!evt.trace_id.is_empty(), "missing trace_id");
    assert!(!evt.span_id.is_empty(), "missing span_id");
    assert!(evt.timestamp > 0, "missing timestamp");
    assert_eq!(evt.level, LogLevel::Info, "missing level");
    assert_eq!(evt.source_module, "feishu", "missing source_module");
    assert_eq!(evt.event_type, "outbound.render", "missing event_type");
    assert!(evt.payload.is_object(), "missing payload");
}

/// outbound.send JSONL contains all required fields.
#[tokio::test]
async fn test_outbound_send_event_structure() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    set_test_trace_id(&adapter, "test-send-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let output = closeclaw_common::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "structure send"}}),
    };
    let _ = plugin.send(&output, "ou_peer", None, None).await;

    let events = wait_for_event_type(temp_dir.path(), "outbound.send").await;
    assert!(!events.is_empty(), "outbound.send event should exist");
    let evt = &events[0];

    assert!(!evt.trace_id.is_empty(), "missing trace_id");
    assert!(!evt.span_id.is_empty(), "missing span_id");
    assert!(evt.timestamp > 0, "missing timestamp");
    assert_eq!(evt.level, LogLevel::Info, "missing level");
    assert_eq!(evt.source_module, "feishu", "missing source_module");
    assert_eq!(evt.event_type, "outbound.send", "missing event_type");
    assert!(evt.payload.is_object(), "missing payload");
}

/// send failure (success=false) still records JSONL event.
#[tokio::test]
async fn test_send_failure_records_event() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    set_test_trace_id(&adapter, "test-send-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    // Unsupported msg_type triggers an error path but send still logs
    let output = closeclaw_common::RenderedOutput {
        msg_type: "unsupported_type".into(),
        payload: serde_json::json!({}),
    };
    let result = plugin.send(&output, "ou_peer", None, None).await;
    assert!(result.is_err(), "unsupported type should error");

    let events = wait_for_event_type(temp_dir.path(), "outbound.send").await;
    assert!(!events.is_empty(), "send failure should still write JSONL");
    let evt = &events[0];
    assert_eq!(evt.payload["success"], false);
}

// ===========================================================================
// emit_debug_event boundaries
// ===========================================================================

/// emit_debug_event with debug_log set but no trace_id in last_metadata:
/// the skip branch must not spawn a writer — no JSONL event is written.
#[tokio::test]
async fn test_render_without_trace_id_skips_event() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_adapter("unused-cli"));
    // Deliberately no set_test_trace_id: trace context is missing here.
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    // Multiline text triggers card rendering path → emit_debug_event is called.
    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let _ = plugin.render(&blocks, None);

    // Skip branch spawns no task at all; give any stray task a short window
    // to run, then assert nothing reached the JSONL file.
    // G1 豁免：负向断言 settle window（确认无写入，非等待异步事件到达）。
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let events = read_events(temp_dir.path()).await;
    assert!(
        events.is_empty(),
        "event without trace_id must be skipped, but got JSONL: {:?}",
        events
    );
}
