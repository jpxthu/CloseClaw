//! Unit tests for FeishuPlugin debug_log framework integration (Step 1.4).
//!
//! Test dimensions:
//! 1. Normal path: debug_log set → parse_inbound/render/send write JSONL
//! 2. No debug_log path: None doesn't panic
//! 3. trace_id propagation: parse → last_metadata → render/send read same
//! 4. Timing accuracy: durations are non-negative
//! 5. Event structure completeness: JSONL has all required fields

use super::adapter::FeishuAdapter;
use super::FeishuPlugin;
use crate::media_store::MediaStore;
use crate::IMPlugin;
use closeclaw_common::processor::ContentBlock;
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel};
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

/// Create a test MediaStore rooted in a temp directory.
fn make_test_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("tmp dir");
    Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"))
}

fn make_test_adapter() -> FeishuAdapter {
    FeishuAdapter::new("test_profile".to_string(), make_test_media_store())
}

async fn make_debug_log(temp_dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 1,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.expect("DebugLog::new failed")
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

/// Read all JSONL lines from the first .jsonl file in the directory.
fn read_jsonl_lines(dir: &std::path::Path) -> Vec<String> {
    let entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
        })
        .collect();
    if entries.is_empty() {
        return vec![];
    }
    let content = std::fs::read_to_string(entries[0].path()).unwrap();
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

/// Wait for a JSONL line to appear (handles async spawn timing).
async fn wait_for_jsonl_lines(dir: &std::path::Path, expected: usize) -> Vec<String> {
    for _ in 0..50 {
        let lines = read_jsonl_lines(dir);
        if lines.len() >= expected {
            return lines;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    read_jsonl_lines(dir)
}

// ===========================================================================
// 1. Normal path: debug_log set → parse_inbound/render/send write JSONL
// ===========================================================================

/// parse_inbound with debug_log writes inbound.parse event to JSONL.
#[tokio::test]
async fn test_parse_inbound_writes_jsonl() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_test_adapter());
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = make_text_payload("hello jsonl");
    let _ = plugin.parse_inbound(&payload).await.unwrap();

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    assert!(!lines.is_empty(), "parse_inbound should write JSONL");

    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(parsed["event_type"], "inbound.parse");
    assert_eq!(parsed["payload"]["platform"], "feishu");
    assert_eq!(parsed["payload"]["message_type"], "text");
    assert!(
        parsed["payload"]["parse_duration_ms"].is_number(),
        "parse_duration_ms should be a number"
    );
}

/// render with debug_log writes outbound.render event to JSONL.
/// Uses multiline text to trigger the card rendering path (bypasses early return).
#[tokio::test]
async fn test_render_writes_jsonl() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_test_adapter());
    set_test_trace_id(&adapter, "test-render-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    // Multiline text triggers card rendering path (bypasses early return).
    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let _ = plugin.render(&blocks, None);

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    assert!(!lines.is_empty(), "render should write JSONL");

    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(parsed["event_type"], "outbound.render");
    assert_eq!(parsed["payload"]["platform"], "feishu");
    assert!(
        parsed["payload"]["msg_type"].is_string(),
        "msg_type should be a string"
    );
    assert!(
        parsed["payload"]["render_duration_ms"].is_number(),
        "render_duration_ms should be a number"
    );
}

/// send with debug_log writes outbound.send event to JSONL.
#[tokio::test]
async fn test_send_writes_jsonl() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_test_adapter());
    set_test_trace_id(&adapter, "test-send-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let output = closeclaw_common::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "send test"}}),
    };
    let result = plugin.send(&output, "ou_peer", None, None).await;
    assert!(result.is_ok());

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    assert!(!lines.is_empty(), "send should write JSONL");

    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(parsed["event_type"], "outbound.send");
    assert_eq!(parsed["payload"]["platform"], "feishu");
    assert_eq!(parsed["payload"]["peer_id"], "ou_peer");
    assert_eq!(parsed["payload"]["msg_type"], "text");
    assert!(
        parsed["payload"]["send_duration_ms"].is_number(),
        "send_duration_ms should be a number"
    );
    assert!(
        parsed["payload"]["success"].is_boolean(),
        "success should be a boolean"
    );
}

// ===========================================================================
// 2. No debug_log path: None doesn't panic
// ===========================================================================

/// parse_inbound without debug_log works normally (no panic).
#[tokio::test]
async fn test_parse_inbound_without_debug_log_no_panic() {
    let adapter = Arc::new(make_test_adapter());
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
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);

    let blocks = vec![ContentBlock::Text("plain".into())];
    let output = plugin.render(&blocks, None);
    assert_eq!(output.msg_type, "text");
}

/// send without debug_log works normally (no panic).
#[tokio::test]
async fn test_send_without_debug_log_no_panic() {
    let adapter = Arc::new(make_test_adapter());
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
    let adapter = Arc::new(make_test_adapter());
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
    let adapter = Arc::new(make_test_adapter());
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

    let lines = wait_for_jsonl_lines(temp_dir.path(), 2).await;
    let render_line = lines
        .iter()
        .find(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .map(|v| v["event_type"] == "outbound.render")
                .unwrap_or(false)
        })
        .expect("render event should exist");

    let parsed: serde_json::Value = serde_json::from_str(render_line).unwrap();
    assert_eq!(
        parsed["trace_id"].as_str(),
        Some(trace_id.as_str()),
        "render trace_id should match parse_inbound trace_id"
    );
}

/// parse_inbound generates trace_id in last_metadata; send reads it.
#[tokio::test]
async fn test_trace_id_propagation_send() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_test_adapter());
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

    let lines = wait_for_jsonl_lines(temp_dir.path(), 2).await;
    let send_line = lines
        .iter()
        .find(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .map(|v| v["event_type"] == "outbound.send")
                .unwrap_or(false)
        })
        .expect("send event should exist");

    let parsed: serde_json::Value = serde_json::from_str(send_line).unwrap();
    assert_eq!(
        parsed["trace_id"].as_str(),
        Some(trace_id.as_str()),
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
    let adapter = Arc::new(make_test_adapter());
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = make_text_payload("timing parse");
    let _ = plugin.parse_inbound(&payload).await.unwrap();

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let ms = parsed["payload"]["parse_duration_ms"]
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
    let adapter = Arc::new(make_test_adapter());
    set_test_trace_id(&adapter, "test-render-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let _ = plugin.render(&blocks, None);

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    assert!(!lines.is_empty());
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let ms = parsed["payload"]["render_duration_ms"]
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
    let adapter = Arc::new(make_test_adapter());
    set_test_trace_id(&adapter, "test-send-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let output = closeclaw_common::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "timing send"}}),
    };
    let _ = plugin.send(&output, "ou_peer", None, None).await;

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let ms = parsed["payload"]["send_duration_ms"]
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
// ===========================================================================

/// inbound.parse JSONL contains all required fields.
#[tokio::test]
async fn test_inbound_parse_event_structure() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_test_adapter());
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let payload = make_text_payload("structure test");
    let _ = plugin.parse_inbound(&payload).await.unwrap();

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();

    // Required top-level fields
    assert!(parsed["trace_id"].is_string(), "missing trace_id");
    assert!(parsed["span_id"].is_string(), "missing span_id");
    assert!(parsed["timestamp"].is_number(), "missing timestamp");
    assert!(parsed["level"].is_string(), "missing level");
    assert!(parsed["source_module"].is_string(), "missing source_module");
    assert!(parsed["event_type"].is_string(), "missing event_type");
    assert!(parsed["payload"].is_object(), "missing payload");
    assert_eq!(parsed["source_module"], "feishu");
}

/// outbound.render JSONL contains all required fields.
/// Uses multiline text to trigger the card rendering path.
#[tokio::test]
async fn test_outbound_render_event_structure() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_test_adapter());
    set_test_trace_id(&adapter, "test-render-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let _ = plugin.render(&blocks, None);

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    assert!(!lines.is_empty());
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();

    assert!(parsed["trace_id"].is_string(), "missing trace_id");
    assert!(parsed["span_id"].is_string(), "missing span_id");
    assert!(parsed["timestamp"].is_number(), "missing timestamp");
    assert!(parsed["level"].is_string(), "missing level");
    assert!(parsed["source_module"].is_string(), "missing source_module");
    assert!(parsed["event_type"].is_string(), "missing event_type");
    assert!(parsed["payload"].is_object(), "missing payload");
    assert_eq!(parsed["source_module"], "feishu");
}

/// outbound.send JSONL contains all required fields.
#[tokio::test]
async fn test_outbound_send_event_structure() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_test_adapter());
    set_test_trace_id(&adapter, "test-send-trace").await;
    let mut plugin = FeishuPlugin::new(adapter);
    plugin.set_debug_log(Arc::new(debug_log));

    let output = closeclaw_common::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "structure send"}}),
    };
    let _ = plugin.send(&output, "ou_peer", None, None).await;

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();

    assert!(parsed["trace_id"].is_string(), "missing trace_id");
    assert!(parsed["span_id"].is_string(), "missing span_id");
    assert!(parsed["timestamp"].is_number(), "missing timestamp");
    assert!(parsed["level"].is_string(), "missing level");
    assert!(parsed["source_module"].is_string(), "missing source_module");
    assert!(parsed["event_type"].is_string(), "missing event_type");
    assert!(parsed["payload"].is_object(), "missing payload");
    assert_eq!(parsed["source_module"], "feishu");
}

/// send failure (success=false) still records JSONL event.
#[tokio::test]
async fn test_send_failure_records_event() {
    let temp_dir = TempDir::new().unwrap();
    let debug_log = make_debug_log(&temp_dir).await;
    let adapter = Arc::new(make_test_adapter());
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

    let lines = wait_for_jsonl_lines(temp_dir.path(), 1).await;
    assert!(!lines.is_empty(), "send failure should still write JSONL");

    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(parsed["event_type"], "outbound.send");
    assert_eq!(parsed["payload"]["success"], false);
}
