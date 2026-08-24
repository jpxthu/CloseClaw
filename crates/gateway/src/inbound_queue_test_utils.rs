//! Shared test utilities for inbound queue tests.

use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig, InboundRequest};
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;
use tempfile::TempDir;

use super::inbound_queue::QueuedInbound;

pub fn make_raw_payload(text: &str) -> Vec<u8> {
    serde_json::json!({
        "header": {
            "event_id": "ev_test",
            "event_type": "im.message.receive_v1",
            "create_time": "1700000000000",
            "token": "t",
            "app_id": "a"
        },
        "event": {
            "sender": {
                "sender_id": {
                    "open_id": "u1"
                },
                "sender_type": "user",
                "tenant_key": "tk"
            },
            "message": {
                "message_id": "m1",
                "root_id": "",
                "parent_id": "",
                "create_time": "1700000000000",
                "chat_id": "p1",
                "chat_type": "p2p",
                "message_type": "text",
                "content": format!("{{\"text\":\"{}\"}}", text)
            }
        }
    })
    .to_string()
    .into_bytes()
}

pub fn make_request(content: &str) -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        raw_payload: make_raw_payload(content),
        peer_id: "p1".into(),
        trace_id: String::new(),
    }
}

/// Wrap an `InboundRequest` in a [`QueuedInbound`].
/// For tests that only care about the request payload.
pub fn queued(request: InboundRequest) -> QueuedInbound {
    QueuedInbound { request }
}

pub fn make_gateway() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 4,
        inbound_wal_dir: None,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    Arc::new(Gateway::new(config, sm))
}

// ── Shared debug-log helpers ─────────────────────────────────────────────────

/// Create a [`DebugLog`] writing to a temp directory with Trace-level logging.
pub async fn make_debug_log(temp_dir: &TempDir) -> DebugLog {
    let config = DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 1,
        redaction_patterns: vec![],
    };
    DebugLog::new(config).await.expect("DebugLog::new failed")
}

/// Read all [`LogEvent`] entries from JSONL files in `dir`.
pub async fn read_events(dir: &std::path::Path) -> Vec<closeclaw_debug_log::LogEvent> {
    let mut events = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
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

/// Poll [`read_events`] until events appear, up to 3 seconds.
pub async fn read_events_with_timeout(dir: &std::path::Path) -> Vec<closeclaw_debug_log::LogEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let events = read_events(dir).await;
        if !events.is_empty() || tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Filter events by event type.
pub fn filter_events_by_type<'a>(
    events: &'a [closeclaw_debug_log::LogEvent],
    event_type: &str,
) -> Vec<&'a closeclaw_debug_log::LogEvent> {
    events
        .iter()
        .filter(|e| e.event_type == event_type)
        .collect()
}

/// Filter events by event type and trace_id.
pub fn filter_events_by_type_and_trace<'a>(
    events: &'a [closeclaw_debug_log::LogEvent],
    event_type: &str,
    trace_id: &str,
) -> Vec<&'a closeclaw_debug_log::LogEvent> {
    events
        .iter()
        .filter(|e| e.event_type == event_type && e.trace_id == trace_id)
        .collect()
}

/// Poll until the WAL at `wal_dir` has zero pending entries, up to 5 seconds.
/// Panics if deadline is exceeded.
pub async fn wait_wal_empty(wal_dir: &std::path::Path) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let wal = super::inbound_wal::InboundWal::open(wal_dir).unwrap();
        let remaining = wal.load_all().unwrap();
        if remaining.is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "WAL should be empty after processing, got {} entries",
            remaining.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
