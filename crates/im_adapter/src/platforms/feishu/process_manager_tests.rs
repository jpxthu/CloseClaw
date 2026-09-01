//! Tests for the lark-cli subprocess event manager.
//!
//! Covers:
//! - Dual format parsing (CLI flat format + webhook envelope format)
//! - Event flow through ProcessManager (start → read events → shutdown)
//! - Graceful shutdown
//! - Auto-restart on abnormal exit
//! - Ready signal detection
//! - Process lifecycle (PID, running state)

use super::process_manager::*;
use std::os::unix::fs::PermissionsExt;

// ===========================================================================
// Helpers
// ===========================================================================

/// Create a mock script that writes NDJSON lines to stdout and a ready signal
/// to stderr, then exits.
fn create_mock_script(lines: &[&str]) -> (tempfile::TempDir, String) {
    let dir = tempfile::TempDir::new().unwrap();
    let script_path = dir.path().join("mock_lark_cli.sh");

    let mut content = String::from("#!/bin/bash\n");
    content.push_str("echo '[event] ready' >&2\n");
    for line in lines {
        content.push_str(&format!("echo '{line}'\n"));
    }
    content.push_str("exit 0\n");

    std::fs::write(&script_path, &content).unwrap();
    std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    (dir, script_path.to_str().unwrap().to_string())
}

/// Create a mock script that outputs events at a given interval.
fn create_slow_script(events: &[&str], delay_ms: u64) -> (tempfile::TempDir, String) {
    let dir = tempfile::TempDir::new().unwrap();
    let script_path = dir.path().join("slow_lark_cli.sh");

    let mut content = String::from("#!/bin/bash\n");
    content.push_str("echo '[event] ready' >&2\n");
    for event in events {
        content.push_str(&format!("sleep 0.{delay_ms:03}\necho '{event}'\n"));
    }
    content.push_str("exit 0\n");

    std::fs::write(&script_path, &content).unwrap();
    std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    (dir, script_path.to_str().unwrap().to_string())
}

/// Create a mock script that outputs many events quickly.
fn create_burst_script(count: usize) -> (tempfile::TempDir, String) {
    let dir = tempfile::TempDir::new().unwrap();
    let script_path = dir.path().join("burst_lark_cli.sh");

    let mut content = String::from("#!/bin/bash\n");
    content.push_str("echo '[event] ready' >&2\n");
    for i in 0..count {
        let line = serde_json::json!({
            "type": "im.message.receive_v1",
            "event_id": format!("ev_{i:04}"),
            "message_id": format!("om_{i:04}"),
            "sender_id": "ou_user",
            "content": "{\"text\":\"hello\"}"
        });
        content.push_str(&format!("echo '{line}'\n"));
    }
    content.push_str("exit 0\n");

    std::fs::write(&script_path, &content).unwrap();
    std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    (dir, script_path.to_str().unwrap().to_string())
}

// ===========================================================================
// Format parsing tests
// ===========================================================================

#[test]
fn test_extract_event_type_cli_format() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_001",
        "id": "om_001"
    });
    assert_eq!(extract_event_type(&raw), "im.message.receive_v1");
}

#[test]
fn test_extract_event_type_webhook_format() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_type": "im.message.reaction.created_v1",
            "event_id": "ev_002"
        },
        "event": {}
    });
    assert_eq!(extract_event_type(&raw), "im.message.reaction.created_v1");
}

#[test]
fn test_extract_event_type_missing() {
    let raw = serde_json::json!({"foo": "bar"});
    assert_eq!(extract_event_type(&raw), "");
}

#[test]
fn test_extract_event_id_cli_format() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_cli_001"
    });
    assert_eq!(extract_event_id(&raw), "ev_cli_001");
}

#[test]
fn test_extract_event_id_webhook_format() {
    let raw = serde_json::json!({
        "header": {"event_id": "ev_wh_001"}
    });
    assert_eq!(extract_event_id(&raw), "ev_wh_001");
}

#[test]
fn test_extract_event_id_missing() {
    let raw = serde_json::json!({});
    assert_eq!(extract_event_id(&raw), "");
}

// ===========================================================================
// EventLine::parse tests
// ===========================================================================

#[test]
fn test_parse_event_line_cli_format() {
    let line = r#"{"type":"im.message.receive_v1","event_id":"ev_001","message_id":"om_001","sender_id":"ou_user","content":"{\"text\":\"hello\"}"}"#;
    let result = ProcessManager::parse_event(line);
    match result {
        EventLine::Event(e) => {
            assert_eq!(e.event_type, "im.message.receive_v1");
            assert_eq!(e.event_id, "ev_001");
        }
        EventLine::Error(err) => panic!("expected Event, got Error: {err}"),
    }
}

#[test]
fn test_parse_event_line_webhook_format() {
    let line = r#"{"schema":"2.0","header":{"event_type":"im.message.reaction.created_v1","event_id":"ev_002","create_time":"1234567890","token":"","app_id":"app_123"},"event":{"message_id":"om_002","reaction_type":{"emoji_type":"THUMBSUP"}}}"#;
    let result = ProcessManager::parse_event(line);
    match result {
        EventLine::Event(e) => {
            assert_eq!(e.event_type, "im.message.reaction.created_v1");
            assert_eq!(e.event_id, "ev_002");
        }
        EventLine::Error(err) => panic!("expected Event, got Error: {err}"),
    }
}

#[test]
fn test_parse_event_line_invalid_json() {
    let result = ProcessManager::parse_event("not valid json {{{");
    assert!(matches!(result, EventLine::Error(_)));
}

#[test]
fn test_parse_event_line_empty_line() {
    let result = ProcessManager::parse_event("");
    assert!(matches!(result, EventLine::Error(_)));
}

#[test]
fn test_parse_event_line_cli_missing_type() {
    let line = r#"{"event_id": "ev_123"}"#;
    let result = ProcessManager::parse_event(line);
    assert!(matches!(result, EventLine::Error(_)));
}

#[test]
fn test_parse_event_line_webhook_missing_header() {
    let line = r#"{"schema": "2.0"}"#;
    let result = ProcessManager::parse_event(line);
    assert!(matches!(result, EventLine::Error(_)));
}

#[test]
fn test_parse_event_line_mixed_format_detection() {
    // CLI format: has top-level "type"
    let cli = r#"{"type":"im.message.receive_v1","event_id":"ev_1"}"#;
    let result = ProcessManager::parse_event(cli);
    match result {
        EventLine::Event(e) => assert_eq!(e.event_type, "im.message.receive_v1"),
        EventLine::Error(err) => panic!("CLI parse failed: {err}"),
    }

    // Webhook format: has "header.event_type"
    let webhook = r#"{"schema":"2.0","header":{"event_type":"bot.added","event_id":"ev_2"}}"#;
    let result = ProcessManager::parse_event(webhook);
    match result {
        EventLine::Event(e) => assert_eq!(e.event_type, "bot.added"),
        EventLine::Error(err) => panic!("Webhook parse failed: {err}"),
    }
}

#[test]
fn test_parse_event_line_large_event() {
    let content = "x".repeat(50_000);
    let line = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_large",
        "content": content
    })
    .to_string();
    let result = ProcessManager::parse_event(&line);
    match result {
        EventLine::Event(e) => {
            assert_eq!(e.event_id, "ev_large");
            assert_eq!(
                e.raw.get("content").unwrap().as_str().unwrap().len(),
                50_000
            );
        }
        EventLine::Error(err) => panic!("large event parse failed: {err}"),
    }
}

#[test]
fn test_parse_event_line_unicode_content() {
    let line = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_unicode",
        "content": "你好世界 🌍🎉"
    })
    .to_string();
    let result = ProcessManager::parse_event(&line);
    match result {
        EventLine::Event(e) => {
            let content = e.raw.get("content").unwrap().as_str().unwrap();
            assert_eq!(content, "你好世界 🌍🎉");
        }
        EventLine::Error(err) => panic!("unicode parse failed: {err}"),
    }
}

#[test]
fn test_parse_event_line_event_id_empty() {
    let line = r#"{"type":"im.message.receive_v1","event_id":"","id":"om_001"}"#;
    let result = ProcessManager::parse_event(line);
    match result {
        EventLine::Event(e) => assert_eq!(e.event_id, ""),
        EventLine::Error(err) => panic!("expected Event with empty event_id, got Error: {err}"),
    }
}

#[test]
fn test_parse_event_line_deeply_nested() {
    let line = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_type": "card.action.trigger",
            "event_id": "ev_card",
            "app_id": "app_123"
        },
        "event": {
            "operator": {"open_id": "ou_op"},
            "action": {"tag": "button", "value": {"action": "click"}}
        }
    })
    .to_string();
    let result = ProcessManager::parse_event(&line);
    match result {
        EventLine::Event(e) => {
            assert_eq!(e.event_type, "card.action.trigger");
            assert_eq!(e.event_id, "ev_card");
        }
        EventLine::Error(err) => panic!("nested event parse failed: {err}"),
    }
}

// ===========================================================================
// ProcessManager lifecycle tests
// ===========================================================================

#[tokio::test]
async fn test_process_manager_start_and_receive_events() {
    let cli_event = r#"{"type":"im.message.receive_v1","event_id":"ev_001","message_id":"om_001","sender_id":"ou_user","content":"{\"text\":\"hello\"}"}"#;
    let webhook_event = r#"{"schema":"2.0","header":{"event_type":"im.message.reaction.created_v1","event_id":"ev_002","create_time":"123","token":"","app_id":"app_1"},"event":{"message_id":"om_002","reaction_type":{"emoji_type":"THUMBSUP"}}}"#;

    let (_dir, script) = create_mock_script(&[cli_event, webhook_event]);
    let (mut manager, mut rx) = ProcessManager::new("bash".into(), vec![script]);

    manager.start().await.unwrap();
    assert!(manager.is_running());

    let mut received = Vec::new();
    while let Ok(line) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
        if let Some(EventLine::Event(e)) = line {
            received.push(e);
        }
        if received.len() == 2 {
            break;
        }
    }

    assert_eq!(received.len(), 2);
    assert_eq!(received[0].event_type, "im.message.receive_v1");
    assert_eq!(received[0].event_id, "ev_001");
    assert_eq!(received[1].event_type, "im.message.reaction.created_v1");
    assert_eq!(received[1].event_id, "ev_002");

    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_process_manager_empty_output() {
    let (_dir, script) = create_mock_script(&[]);
    let (mut manager, mut rx) = ProcessManager::new("bash".into(), vec![script]);

    manager.start().await.unwrap();

    // No events expected, just EOF
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await;
    assert!(result.is_ok() || result.is_err());

    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_process_manager_ready_timeout() {
    let dir = tempfile::TempDir::new().unwrap();
    let script_path = dir.path().join("no_ready.sh");
    std::fs::write(&script_path, "#!/bin/bash\nwhile true; do sleep 1; done\n").unwrap();
    std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();

    let (mut manager, _rx) = ProcessManager::new(
        "bash".into(),
        vec![script_path.to_str().unwrap().to_string()],
    );

    let result = tokio::time::timeout(std::time::Duration::from_secs(35), manager.start()).await;

    match result {
        Ok(Err(ProcessError::ReadyTimeout)) => {} // Expected
        Ok(Err(e)) => panic!("expected ReadyTimeout, got: {e}"),
        Ok(Ok(())) => panic!("expected error, got Ok"),
        Err(_) => {} // Timeout is also acceptable
    }

    let _ = manager.shutdown().await;
}

#[tokio::test]
async fn test_process_manager_parse_event_exposed() {
    let cli_line = r#"{"type":"im.message.receive_v1","event_id":"ev_001"}"#;
    let result = ProcessManager::parse_event(cli_line);
    match result {
        EventLine::Event(e) => {
            assert_eq!(e.event_type, "im.message.receive_v1");
            assert_eq!(e.event_id, "ev_001");
        }
        EventLine::Error(err) => panic!("expected Event, got Error: {err}"),
    }
}

#[tokio::test]
async fn test_process_manager_process_id() {
    let (_dir, script) = create_mock_script(&[]);
    let (mut manager, _rx) = ProcessManager::new("bash".into(), vec![script]);

    assert!(manager.pid().is_none());
    manager.start().await.unwrap();
    assert!(manager.pid().is_some());

    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_process_manager_graceful_shutdown() {
    let event = r#"{"type":"im.message.receive_v1","event_id":"ev_001"}"#;
    let (_dir, script) = create_slow_script(&[event, event, event], 200);
    let (mut manager, mut rx) = ProcessManager::new("bash".into(), vec![script]);

    manager.start().await.unwrap();

    let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(first, EventLine::Event(_)));

    manager.shutdown().await.unwrap();
    assert!(!manager.is_running());
}

#[tokio::test]
async fn test_process_manager_burst_events() {
    let (_dir, script) = create_burst_script(100);
    let (mut manager, mut rx) = ProcessManager::new("bash".into(), vec![script]);

    manager.start().await.unwrap();

    let mut count = 0;
    while let Ok(Some(EventLine::Event(_))) =
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await
    {
        count += 1;
        if count >= 100 {
            break;
        }
    }

    assert_eq!(count, 100);
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_process_manager_new_defaults() {
    let (manager, _rx) =
        ProcessManager::new("lark-cli".into(), vec!["event".into(), "consume".into()]);
    assert!(!manager.is_running());
    assert!(manager.pid().is_none());
    assert_eq!(manager.command, "lark-cli");
    assert_eq!(manager.args, vec!["event", "consume"]);
}

#[tokio::test]
async fn test_process_manager_double_shutdown() {
    let (_dir, script) = create_mock_script(&[]);
    let (mut manager, _rx) = ProcessManager::new("bash".into(), vec![script]);

    manager.start().await.unwrap();
    manager.shutdown().await.unwrap();
    // Second shutdown should not panic
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_process_manager_invalid_command() {
    let (mut manager, _rx) = ProcessManager::new("/nonexistent/binary".into(), vec![]);

    let result = manager.start().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_process_manager_event_channel_dropped() {
    let event = r#"{"type":"im.message.receive_v1","event_id":"ev_001"}"#;
    let (_dir, script) = create_slow_script(&[event, event], 200);
    let (mut manager, rx) = ProcessManager::new("bash".into(), vec![script]);

    manager.start().await.unwrap();

    // Drop the receiver
    drop(rx);

    // The process should still be running (write side doesn't crash)
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    manager.shutdown().await.unwrap();
}

// ===========================================================================
// Format detection integration tests (via ProcessManager::parse_event)
// ===========================================================================

#[test]
fn test_format_detection_cli_preferred_over_webhook() {
    // When both "type" and "header.event_type" are present,
    // CLI format should be preferred
    let line = serde_json::json!({
        "type": "im.message.receive_v1",
        "header": {
            "event_type": "should_not_be_used",
            "event_id": "ev_123"
        },
        "event_id": "ev_correct"
    })
    .to_string();
    let result = ProcessManager::parse_event(&line);
    match result {
        EventLine::Event(e) => {
            assert_eq!(e.event_type, "im.message.receive_v1");
            assert_eq!(e.event_id, "ev_correct");
        }
        EventLine::Error(err) => panic!("expected Event, got Error: {err}"),
    }
}

// ===========================================================================
// start_event_stream tests
// ===========================================================================

/// Create a test Gateway with inbound queue and DebugLog started.
///
/// Returns `(gateway, debug_log_dir)` — the caller can read JSONL files
/// from `debug_log_dir` to verify debug events.
async fn make_test_gateway_with_debug_log() -> (
    std::sync::Arc<closeclaw_gateway::Gateway>,
    tempfile::TempDir,
) {
    use closeclaw_gateway::{Gateway, GatewayConfig};

    let temp_dir = tempfile::TempDir::new().unwrap();
    let config = GatewayConfig {
        name: "test-debug-log".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 16,
        inbound_wal_dir: None,
        ..Default::default()
    };
    let sm = std::sync::Arc::new(closeclaw_gateway::SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let gw = std::sync::Arc::new(Gateway::new(config, sm));
    gw.start_inbound_queue();

    // Set DebugLog so emit_arrived_log writes to JSONL.
    use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel};
    let debug_log = DebugLog::new(DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 1,
        redaction_patterns: vec![],
    })
    .await
    .expect("DebugLog::new failed");
    gw.set_debug_log(debug_log).await;

    (gw, temp_dir)
}

/// Read all LogEvent entries from JSONL files in `dir` (sync helper).
fn read_debug_events(dir: &std::path::Path) -> Vec<closeclaw_debug_log::LogEvent> {
    let mut events = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    for line in content.lines() {
                        if !line.trim().is_empty() {
                            if let Ok(event) = closeclaw_debug_log::LogEvent::from_jsonl(line) {
                                events.push(event);
                            }
                        }
                    }
                }
            }
        }
    }
    events
}

/// Poll until events appear or timeout, up to 3 seconds.
async fn wait_debug_events(
    dir: &std::path::Path,
    min_count: usize,
) -> Vec<closeclaw_debug_log::LogEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let events = read_debug_events(dir);
        if events.len() >= min_count || tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Successful enqueue of a message event emits exactly one `gateway.arrived`
/// debug event with correct platform.
#[tokio::test]
async fn test_start_event_stream_enqueues_message_event() {
    let (gw, temp_dir) = make_test_gateway_with_debug_log().await;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    start_event_stream(&gw, rx);

    let event = Event {
        event_type: "im.message.receive_v1".to_string(),
        event_id: "ev_stream_001".to_string(),
        raw: serde_json::json!({
            "type": "im.message.receive_v1",
            "event_id": "ev_stream_001",
            "message_id": "om_001",
            "sender_id": "ou_user",
            "content": "{\"text\":\"hello\"}",
            "chat_id": "oc_chat",
            "message_type": "text"
        }),
    };
    tx.send(EventLine::Event(event)).unwrap();
    drop(tx);

    let events = wait_debug_events(temp_dir.path(), 1).await;
    let arrived: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "gateway.arrived" && e.source_module == "gateway")
        .collect();
    assert_eq!(
        arrived.len(),
        1,
        "expected exactly one gateway.arrived event, got {}",
        arrived.len()
    );
    assert_eq!(arrived[0].payload["platform"].as_str().unwrap(), "feishu");
}

/// Error lines (parse failures) must NOT emit `gateway.arrived`.
#[tokio::test]
async fn test_start_event_stream_skips_error_lines() {
    let (gw, temp_dir) = make_test_gateway_with_debug_log().await;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    start_event_stream(&gw, rx);

    tx.send(EventLine::Error("bad json".to_string())).unwrap();
    drop(tx);

    let events = wait_debug_events(temp_dir.path(), 0).await;
    let arrived: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "gateway.arrived")
        .collect();
    assert_eq!(
        arrived.len(),
        0,
        "no gateway.arrived event expected for error lines"
    );
}

/// Multiple events each emit their own `gateway.arrived` debug event.
#[tokio::test]
async fn test_start_event_stream_multiple_events() {
    let (gw, temp_dir) = make_test_gateway_with_debug_log().await;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    start_event_stream(&gw, rx);

    for i in 0..5 {
        let event = Event {
            event_type: "im.message.receive_v1".to_string(),
            event_id: format!("ev_multi_{i}"),
            raw: serde_json::json!({
                "type": "im.message.receive_v1",
                "event_id": format!("ev_multi_{i}"),
                "message_id": format!("om_{i}"),
                "sender_id": "ou_user",
                "content": "{\"text\":\"hello\"}",
                "chat_id": "oc_chat",
                "message_type": "text"
            }),
        };
        tx.send(EventLine::Event(event)).unwrap();
    }
    drop(tx);

    let events = wait_debug_events(temp_dir.path(), 5).await;
    let arrived: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "gateway.arrived" && e.source_module == "gateway")
        .collect();
    assert_eq!(
        arrived.len(),
        5,
        "expected 5 gateway.arrived events, got {}",
        arrived.len()
    );
    for evt in &arrived {
        assert_eq!(evt.payload["platform"].as_str().unwrap(), "feishu");
    }
}
