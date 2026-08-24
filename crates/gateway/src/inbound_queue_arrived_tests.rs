//! Tests for `gateway.arrived` debug event emission (Step 1.2).
//!
//! Verifies that `enqueue_inbound` emits exactly one `gateway.arrived`
//! event on successful enqueue, no event when DebugLog is not configured,
//! and no event on queue-full rejection.

use std::sync::Arc;

use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig};
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel};
use closeclaw_session::persistence::ReasoningLevel;
use tempfile::TempDir;

use super::inbound_queue_test_utils::{make_gateway, make_request, queued};

/// Create a DebugLog writing to a temp directory.
async fn make_debug_log(temp_dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 1,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.expect("DebugLog::new failed")
}

/// Read all LogEvent entries from JSONL files in `dir`.
async fn read_events(dir: &std::path::Path) -> Vec<closeclaw_debug_log::LogEvent> {
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

/// Poll `read_events` until events appear, up to 2s.
async fn read_events_with_timeout(dir: &std::path::Path) -> Vec<closeclaw_debug_log::LogEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let events = read_events(dir).await;
        if !events.is_empty() || tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Successful enqueue emits exactly one `gateway.arrived` event.
#[tokio::test]
async fn test_arrived_event_emitted_on_successful_enqueue() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = GatewayConfig {
        name: "test-arrived".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 4,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;
    let _handle = gw.start_inbound_queue();

    let trace_id = "feishu-arrived-test-001";
    let mut req = make_request("arrived-test");
    req.trace_id = trace_id.to_string();

    let result = gw.enqueue_inbound(req).await;
    assert!(result.is_ok(), "enqueue should succeed");

    // Give the spawned debug-log task time to write.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let events = read_events_with_timeout(temp_dir.path()).await;
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
    let evt = &arrived[0];
    assert_eq!(evt.trace_id, trace_id);
    assert_eq!(evt.payload["platform"].as_str().unwrap(), "feishu");
    assert_eq!(evt.payload["peer_id"].as_str().unwrap(), "p1");
    assert_eq!(evt.level, LogLevel::Debug);
}

/// When DebugLog is not configured, `gateway.arrived` must not be emitted.
#[tokio::test]
async fn test_no_debug_log_no_arrived_event() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let gw = make_gateway();
    // No debug_log set — stays None.
    let _handle = gw.start_inbound_queue();

    let mut req = make_request("no-dl-arrived");
    req.trace_id = "feishu-no-dl-arrived-001".to_string();

    let result = gw.enqueue_inbound(req).await;
    assert!(result.is_ok());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let mut has_jsonl = false;
    if let Ok(mut entries) = tokio::fs::read_dir(temp_dir.path()).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl") {
                has_jsonl = true;
                break;
            }
        }
    }
    assert!(!has_jsonl, "no jsonl files expected when DebugLog is None");
}

/// Queue-full path must NOT emit `gateway.arrived` (only `queue.rejected`).
#[tokio::test]
async fn test_queue_full_no_arrived_event() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = GatewayConfig {
        name: "test-arrived-full".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 1,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;
    let handle = gw.start_inbound_queue();

    // Fill queue to capacity (1).
    handle.try_send(queued(make_request("fill"))).unwrap();

    // Enqueue one more — should trigger queue-full path.
    let mut req = make_request("overflow");
    req.trace_id = "feishu-full-arrived-001".to_string();
    let result = gw.enqueue_inbound(req).await;
    assert!(result.is_err(), "queue full should return Err");

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let events = read_events_with_timeout(temp_dir.path()).await;
    let arrived: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "gateway.arrived")
        .collect();
    assert_eq!(
        arrived.len(),
        0,
        "no gateway.arrived event expected on queue-full path"
    );
    // Verify queue.rejected IS emitted (regression check).
    let rejected: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "queue.rejected")
        .collect();
    assert_eq!(
        rejected.len(),
        1,
        "queue.rejected should still be emitted on queue-full"
    );
}

/// Verify that queue.dequeued event is still emitted (regression check).
#[tokio::test]
async fn test_queue_dequeued_event_still_emitted() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = GatewayConfig {
        name: "test-dequeued".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 4,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;
    let _handle = gw.start_inbound_queue();

    let trace_id = "feishu-dequeued-test-001";
    let mut req = make_request("dequeued-test");
    req.trace_id = trace_id.to_string();

    let result = gw.enqueue_inbound(req).await;
    assert!(result.is_ok());

    // Wait for consumer to dequeue.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let events = read_events_with_timeout(temp_dir.path()).await;

    // Verify arrived event.
    let arrived: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "gateway.arrived" && e.source_module == "gateway")
        .collect();
    assert_eq!(arrived.len(), 1, "expected exactly one gateway.arrived");
    assert_eq!(arrived[0].trace_id, trace_id);

    // Verify dequeued event.
    let dequeued: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "queue.dequeued" && e.source_module == "gateway")
        .collect();
    assert_eq!(dequeued.len(), 1, "expected exactly one queue.dequeued");
    assert_eq!(dequeued[0].trace_id, trace_id);
}
