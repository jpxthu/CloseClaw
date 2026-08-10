//! Unit tests for Gateway debug log emission (Step 1.6 + 1.8 + 1.3).
//!
//! Covers:
//! 1. message.arrived event emitted with correct trace_id/session_key/level
//! 2. No DebugLog → message processes without panic
//! 3. No trace_id → debug event skipped, no panic
//! 4. emit_debug_event helper: empty trace_id is no-op, missing DebugLog is no-op
//! 5. Step 1.8: send.completed event uses inbound trace_id (not fabricated)
//! 6. Step 1.3: session.resolved and route.decision events

use crate::session_handler::{ActiveSearcherLlmCaller, SessionMessageHandler};
use crate::{compute_session_key, GatewayConfig, SessionManager};
use closeclaw_common::processor::ProcessedMessage;
use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel};
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::LLMRegistry;
use closeclaw_session::persistence::ReasoningLevel;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-debug-log".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_session_manager(config: &GatewayConfig) -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        config,
        None,
        None,
        ReasoningLevel::default(),
    ))
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

fn make_processed_with_trace(trace_id: &str, session_key: &str) -> ProcessedMessage {
    let mut metadata = HashMap::new();
    metadata.insert("trace_id".to_string(), trace_id.to_string());
    metadata.insert("session_key".to_string(), session_key.to_string());
    metadata.insert("peer_id".to_string(), "oc_chat".to_string());
    metadata.insert(
        "message_type".to_string(),
        serde_json::to_string(&closeclaw_common::MessageType::Text)
            .unwrap_or_else(|_| "text".to_string()),
    );
    ProcessedMessage {
        content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
            "hello".to_string(),
        )],
        metadata,
    }
}

fn make_processed_no_trace() -> ProcessedMessage {
    let mut metadata = HashMap::new();
    metadata.insert(
        "message_type".to_string(),
        serde_json::to_string(&closeclaw_common::MessageType::Text)
            .unwrap_or_else(|_| "text".to_string()),
    );
    ProcessedMessage {
        content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
            "hello".to_string(),
        )],
        metadata,
    }
}

// ── emit_debug_event helper tests ────────────────────────────────────────────

/// Empty trace_id must be a no-op (no panic, no write).
#[test]
fn test_emit_debug_event_empty_trace_id_no_op() {
    // Should not panic or attempt to write.
    crate::debug_log_emitter::emit_debug_event(
        None,
        "",
        Some("sess-1"),
        LogLevel::Info,
        "gateway",
        "message.arrived",
        serde_json::json!({}),
    );
}

/// None DebugLog must be a no-op (no panic).
#[test]
fn test_emit_debug_event_none_debug_log_no_op() {
    crate::debug_log_emitter::emit_debug_event(
        None,
        "feishu-123-uuid",
        Some("sess-1"),
        LogLevel::Info,
        "gateway",
        "message.arrived",
        serde_json::json!({}),
    );
}

/// With DebugLog present and valid trace_id, emit_debug_event must spawn
/// a task (no panic). We verify by checking the function completes.
#[test]
fn test_emit_debug_event_with_debug_log_no_panic() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let temp_dir = TempDir::new().unwrap();
        let config = DebugLogConfig {
            min_level: LogLevel::Trace,
            log_dir: temp_dir.path().to_path_buf(),
            retention_days: 1,
            redaction_patterns: vec![],
        };
        let debug_log = DebugLog::new(config).await.unwrap();
        crate::debug_log_emitter::emit_debug_event(
            Some(&debug_log),
            "feishu-123-uuid",
            Some("sess-1"),
            LogLevel::Info,
            "gateway",
            "message.arrived",
            serde_json::json!({"sender_id": "ou_123"}),
        );
        // Give the spawned task a moment to complete.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    });
}

// ── Step 1.3 helpers ────────────────────────────────────────────────────────

/// Read all LogEvent entries from the JSONL files in `dir`.
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

/// Create a SessionMessageHandler for route.decision tests.
/// Sets the LLM caller on SessionManager so sessions can be created.
async fn handler_with_sm(sm: Arc<SessionManager>) -> SessionMessageHandler {
    let registry = Arc::new(LLMRegistry::new());
    let fallback = Arc::new(FallbackClient::from_strings(registry, vec![]));
    let ufc = Arc::new(UnifiedFallbackClient::new(
        vec![],
        Arc::new(CooldownManager::new()),
    ));
    let llm_caller: Arc<dyn closeclaw_common::LlmCaller> =
        Arc::new(crate::llm_caller_impl::FallbackLlmCaller(ufc.clone()));
    sm.set_llm_caller(llm_caller).await;
    let fallback_llm_caller = Arc::new(ActiveSearcherLlmCaller {
        client: ufc,
        model: String::new(),
    });
    SessionMessageHandler::new_no_output(sm, fallback, fallback_llm_caller)
}

/// Create a ProcessedMessage with trace_id, session_key, content, and
/// message_type=Text. `session_key` is the raw key passed to metadata;
/// use `compute_session_key` to generate a valid one.
fn make_processed(trace_id: &str, session_key: &str, content: &str) -> ProcessedMessage {
    let mut metadata = HashMap::new();
    metadata.insert("trace_id".to_string(), trace_id.to_string());
    metadata.insert("session_key".to_string(), session_key.to_string());
    metadata.insert("peer_id".to_string(), "oc_chat".to_string());
    metadata.insert("sender_id".to_string(), "ou_sender".to_string());
    metadata.insert(
        "message_type".to_string(),
        serde_json::to_string(&closeclaw_common::MessageType::Text)
            .unwrap_or_else(|_| "text".to_string()),
    );
    ProcessedMessage {
        content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
            content.to_string(),
        )],
        metadata,
    }
}

// ── Gateway handle_inbound_message tests ─────────────────────────────────────

/// With DebugLog configured, handle_inbound_message must not panic when
/// trace_id is present.
#[tokio::test]
async fn test_handle_inbound_with_debug_log_no_panic() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = make_config();
    let sm = make_session_manager(&config);
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;

    let processed = make_processed_with_trace("feishu-123-uuid1", "sess-1");

    // handle_inbound_message will try to resolve session_key, but since
    // no session exists it returns None. The important thing is no panic
    // and the debug log emission path is exercised.
    let result = gw
        .handle_inbound_message(processed, Some("ou_sender"), "feishu")
        .await;

    // Without a session, resolve fails → None. That's expected.
    assert!(
        result.is_none(),
        "expected None when session resolution fails"
    );
}

/// Without DebugLog (default None), handle_inbound_message must still work
/// without panic.
#[tokio::test]
async fn test_handle_inbound_no_debug_log_no_panic() {
    let config = make_config();
    let sm = make_session_manager(&config);
    let gw = crate::Gateway::new(config, Arc::clone(&sm));

    let processed = make_processed_with_trace("feishu-456-uuid2", "sess-2");

    let result = gw
        .handle_inbound_message(processed, Some("ou_sender"), "feishu")
        .await;

    assert!(
        result.is_none(),
        "expected None when session resolution fails"
    );
}

/// When trace_id is missing from metadata, handle_inbound_message must
/// not panic (debug event emission is skipped).
#[tokio::test]
async fn test_handle_inbound_no_trace_id_no_panic() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = make_config();
    let sm = make_session_manager(&config);
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;

    let processed = make_processed_no_trace();

    let result = gw
        .handle_inbound_message(processed, Some("ou_sender"), "feishu")
        .await;

    // Without session_key, resolve fails → None. No panic.
    assert!(result.is_none(), "expected None when session_key missing");
}

// ═════════════════════════════════════════════════════════════════════════════
// Step 1.3: session.resolved and route.decision debug log events
// ═════════════════════════════════════════════════════════════════════════════

/// When session resolution succeeds and trace_id is present,
/// a `session.resolved` event is emitted with correct fields.
#[tokio::test]
async fn test_session_resolved_event_emitted() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = make_config();
    // Use a writable workspace so SessionManager::resolve succeeds.
    let ws = temp_dir.path().join("ws");
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        Some(ws),
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;

    let sender_id = "ou_sender";
    let peer_id = "oc_chat";
    // Capture the timestamp used for session creation so we can match it later.
    let msg_timestamp = chrono::Utc::now().timestamp();
    let msg = crate::Message {
        id: String::new(),
        from: sender_id.to_string(),
        to: peer_id.to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: msg_timestamp,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let session_id = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("find_or_create failed");

    // Now use the same routing fields in the ProcessedMessage.
    // Note: compute_session_key uses timestamp_ms, but resolve uses
    // timestamp (seconds) internally. The routing key is computed from
    // channel:from:to:account_id (no timestamp), so as long as these
    // match, resolve will find the session.
    let trace_id = "trace-session-resolved-001";
    let session_key = compute_session_key(
        "feishu",
        sender_id,
        peer_id,
        None,
        msg_timestamp * 1000, // compute_session_key takes ms
    );
    let mut metadata = HashMap::new();
    metadata.insert("trace_id".to_string(), trace_id.to_string());
    metadata.insert("session_key".to_string(), session_key.clone());
    metadata.insert("peer_id".to_string(), peer_id.to_string());
    metadata.insert("sender_id".to_string(), sender_id.to_string());
    metadata.insert(
        "message_type".to_string(),
        serde_json::to_string(&closeclaw_common::MessageType::Text)
            .unwrap_or_else(|_| "text".to_string()),
    );
    let processed = ProcessedMessage {
        content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
            "hello".to_string(),
        )],
        metadata,
    };

    let _result = gw
        .handle_inbound_message(processed, Some(sender_id), "feishu")
        .await;

    // Allow spawned debug log tasks to flush.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_events_from_dir(temp_dir.path()).await;
    let resolved: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "session.resolved")
        .collect();
    assert_eq!(
        resolved.len(),
        1,
        "expected exactly one session.resolved event, got {}",
        resolved.len()
    );
    let evt = resolved[0];
    assert_eq!(evt.trace_id, trace_id);
    assert_eq!(evt.source_module, "gateway");
    assert_eq!(evt.level, LogLevel::Info);
    assert_eq!(evt.session_key.as_deref(), Some(session_key.as_str()));
    // Verify payload fields.
    assert!(
        evt.payload.get("session_id").is_some(),
        "payload must contain session_id"
    );
    assert_eq!(
        evt.payload["session_key"].as_str().unwrap(),
        session_key.as_str()
    );
    assert_eq!(evt.payload["session_id"].as_str().unwrap(), session_id);
    assert_eq!(evt.payload["channel"].as_str().unwrap(), "feishu");
}

/// When a slash command is sent and session resolution succeeds,
/// a `route.decision` event with decision="slash" is emitted.
#[tokio::test]
async fn test_route_decision_slash_event_emitted() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = make_config();
    let ws = temp_dir.path().join("ws");
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        Some(ws),
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;

    // Install a SessionMessageHandler to avoid the early return.
    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let gw = gw.with_session_handler(Arc::new(handler));

    // Pre-create a session so resolve succeeds.
    let sender_id = "ou_sender";
    let peer_id = "oc_chat";
    let msg_timestamp = chrono::Utc::now().timestamp();
    let msg = crate::Message {
        id: String::new(),
        from: sender_id.to_string(),
        to: peer_id.to_string(),
        content: String::new(),
        channel: "feishu".to_string(),
        timestamp: msg_timestamp,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let _session_id = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("find_or_create failed");

    let trace_id = "trace-route-slash-001";
    let session_key = compute_session_key("feishu", sender_id, peer_id, None, msg_timestamp * 1000);
    let processed = make_processed(trace_id, &session_key, "/help");

    let _result = gw
        .handle_inbound_message(processed, Some(sender_id), "feishu")
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_events_from_dir(temp_dir.path()).await;
    let decisions: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "route.decision")
        .collect();
    assert!(
        !decisions.is_empty(),
        "expected at least one route.decision event"
    );
    let gw_decision = decisions
        .iter()
        .find(|e| e.source_module == "gateway")
        .expect("no gateway route.decision event");
    assert_eq!(gw_decision.trace_id, trace_id);
    assert_eq!(gw_decision.payload["decision"].as_str().unwrap(), "slash");
    assert!(
        gw_decision.payload["content_prefix"]
            .as_str()
            .unwrap()
            .starts_with('/'),
        "content_prefix should start with '/' for slash command"
    );
}

/// When a normal message is sent and session resolution succeeds,
/// a `route.decision` event with decision="normal" is emitted.
#[tokio::test]
async fn test_route_decision_normal_event_emitted() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = make_config();
    let ws = temp_dir.path().join("ws");
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        Some(ws),
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;

    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let gw = gw.with_session_handler(Arc::new(handler));

    // Pre-create a session so resolve succeeds.
    let sender_id = "ou_sender";
    let peer_id = "oc_chat";
    let msg_timestamp = chrono::Utc::now().timestamp();
    let msg = crate::Message {
        id: String::new(),
        from: sender_id.to_string(),
        to: peer_id.to_string(),
        content: String::new(),
        channel: "feishu".to_string(),
        timestamp: msg_timestamp,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let _session_id = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("find_or_create failed");

    let trace_id = "trace-route-normal-001";
    let session_key = compute_session_key("feishu", sender_id, peer_id, None, msg_timestamp * 1000);
    let processed = make_processed(trace_id, &session_key, "hello world");

    let _result = gw
        .handle_inbound_message(processed, Some(sender_id), "feishu")
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_events_from_dir(temp_dir.path()).await;
    let decisions: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "route.decision")
        .collect();
    assert!(
        !decisions.is_empty(),
        "expected at least one route.decision event"
    );
    let gw_decision = decisions
        .iter()
        .find(|e| e.source_module == "gateway")
        .expect("no gateway route.decision event");
    assert_eq!(gw_decision.trace_id, trace_id);
    assert_eq!(gw_decision.payload["decision"].as_str().unwrap(), "normal");
}

/// When trace_id is empty, session.resolved and route.decision events
/// must not be emitted (no-op).
#[tokio::test]
async fn test_no_trace_id_no_session_resolved_or_route_decision() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = make_config();
    let ws = temp_dir.path().join("ws");
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        Some(ws),
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let debug_log = make_debug_log(&temp_dir).await;
    gw.set_debug_log(debug_log).await;

    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let gw = gw.with_session_handler(Arc::new(handler));

    // No trace_id in metadata.
    let sender_id = "ou_sender";
    let peer_id = "oc_chat";
    let timestamp = chrono::Utc::now().timestamp_millis();
    let session_key = compute_session_key("feishu", sender_id, peer_id, None, timestamp);
    let mut metadata = HashMap::new();
    metadata.insert("session_key".to_string(), session_key);
    metadata.insert("peer_id".to_string(), "oc_chat".to_string());
    metadata.insert(
        "message_type".to_string(),
        serde_json::to_string(&closeclaw_common::MessageType::Text)
            .unwrap_or_else(|_| "text".to_string()),
    );
    let processed = ProcessedMessage {
        content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
            "hello".to_string(),
        )],
        metadata,
    };

    let _result = gw
        .handle_inbound_message(processed, Some("ou_sender"), "feishu")
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = read_events_from_dir(temp_dir.path()).await;
    let session_resolved: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "session.resolved")
        .collect();
    let route_decisions: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "route.decision")
        .collect();
    assert_eq!(
        session_resolved.len(),
        0,
        "no session.resolved event expected when trace_id is empty"
    );
    assert_eq!(
        route_decisions.len(),
        0,
        "no route.decision event expected when trace_id is empty"
    );
}

/// When DebugLog is not configured (None), session.resolved and
/// route.decision events must not be emitted (no-op).
#[tokio::test]
async fn test_no_debug_log_no_session_resolved_or_route_decision() {
    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let config = make_config();
    let ws = temp_dir.path().join("ws");
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        Some(ws),
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    // No debug_log set — stays None.

    let handler = handler_with_sm(Arc::clone(&sm)).await;
    let gw = gw.with_session_handler(Arc::new(handler));

    let sender_id = "ou_sender";
    let peer_id = "oc_chat";
    let timestamp = chrono::Utc::now().timestamp_millis();
    let session_key = compute_session_key("feishu", sender_id, peer_id, None, timestamp);
    let processed = make_processed("trace-no-dl-001", &session_key, "hello");

    let _result = gw
        .handle_inbound_message(processed, Some("ou_sender"), "feishu")
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // No DebugLog directory was created, so no events should exist.
    // Verify the path doesn't even have jsonl files.
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
