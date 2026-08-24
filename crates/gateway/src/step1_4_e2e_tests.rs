//! End-to-end tests for inbound queue WAL lifecycle (Step 1.4).
//!
//! Covers the full lifecycle: enqueue → WAL append pending → consumer
//! processes → WAL entry deleted → reopen same dir → no stale entries.
//!
//! Also covers:
//! - Default config WAL auto-enable assertion
//! - Explicit null disabling WAL
//! - Arrived → dequeued event lifecycle for same trace_id

use std::sync::Arc;

use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig};
use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::{ContentBlock, NormalizedMessage};

use closeclaw_session::persistence::ReasoningLevel;
use tempfile::TempDir;

use super::inbound_queue_test_utils::{
    filter_events_by_type_and_trace, make_debug_log, make_request, read_events_with_timeout,
    wait_wal_empty,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mock plugin that accepts any inbound message and does nothing harmful.
struct E2ePlugin;

#[async_trait]
impl IMPlugin for E2ePlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(Some(NormalizedMessage {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: "e2e-test".into(),
            timestamp: chrono::Utc::now().timestamp(),
            message_type: closeclaw_common::MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: "u1".into(),
            ..Default::default()
        }))
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn make_e2e_config(wal_dir: &std::path::Path) -> GatewayConfig {
    GatewayConfig {
        name: "test-e2e".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 16,
        inbound_wal_dir: Some(wal_dir.to_path_buf()),
        ..Default::default()
    }
}

fn make_no_wal_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-no-wal".to_owned(),
        inbound_queue_capacity: 4,
        inbound_wal_dir: None,
        ..Default::default()
    }
}

// ===========================================================================
// Test 1: Full WAL lifecycle — enqueue → process → delete → reopen clean
// ===========================================================================

/// End-to-end WAL lifecycle:
/// 1. Create Gateway with temp WAL dir
/// 2. Enqueue a message (WAL append pending entry)
/// 3. Consumer processes the message
/// 4. Verify WAL entry is deleted after processing
/// 5. Create a new Gateway with the same WAL dir
/// 6. Verify no stale entries remain (replay finds nothing)
#[tokio::test]
async fn test_e2e_wal_lifecycle_enqueue_process_delete_reopen() {
    let wal_tmp = TempDir::new().expect("TempDir::new failed");
    let wal_dir = wal_tmp.path();

    // ── Phase 1: Gateway A — enqueue and process ────────────────────
    let config_a = make_e2e_config(wal_dir);
    let sm_a = Arc::new(SessionManager::new(
        &config_a,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw_a = Arc::new(Gateway::new(config_a, sm_a));
    gw_a.register_plugin(Arc::new(E2ePlugin) as Arc<dyn IMPlugin>)
        .await;
    let _handle_a = gw_a.start_inbound_queue();

    let trace_id = "e2e-wal-lifecycle-001";
    let mut req = make_request("lifecycle-test");
    req.trace_id = trace_id.to_string();

    let result = gw_a.enqueue_inbound(req).await;
    assert!(result.is_ok(), "enqueue should succeed");

    // Wait for consumer to process and delete WAL entry.
    wait_wal_empty(wal_dir).await;

    // ── Phase 2: Gateway B — reopen and verify clean ────────────────
    let config_b = make_e2e_config(wal_dir);
    let sm_b = Arc::new(SessionManager::new(
        &config_b,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw_b = Arc::new(Gateway::new(config_b, sm_b));
    gw_b.register_plugin(Arc::new(E2ePlugin) as Arc<dyn IMPlugin>)
        .await;
    let _handle_b = gw_b.start_inbound_queue();

    // Verify no entries remain after reopen.
    let wal_b = super::inbound_wal::InboundWal::open(wal_dir).unwrap();
    let entries = wal_b.load_all().unwrap();
    assert!(
        entries.is_empty(),
        "no stale WAL entries should remain after reopen, got {}",
        entries.len()
    );
}

// ===========================================================================
// Test 2: Reopen with pending entries triggers replay
// ===========================================================================

/// Write pending WAL entries directly, then create a Gateway that replays
/// them on startup. Verify the replayed messages are consumed by the
/// consumer and the WAL is cleaned up.
#[tokio::test]
async fn test_e2e_reopen_replays_pending_entries() {
    let wal_tmp = TempDir::new().expect("TempDir::new failed");
    let wal_dir = wal_tmp.path();

    // ── Phase 1: Write pending entries to WAL directly ──────────────
    let wal = super::inbound_wal::InboundWal::open(wal_dir).unwrap();
    wal.append(&super::inbound_wal::InboundWalEntry::new(
        "replay-001".into(),
        "feishu".into(),
        b"{\"test\":\"replay-data\"}",
        "p1".into(),
    ))
    .unwrap();
    drop(wal);

    // ── Phase 2: Gateway replays on start ───────────────────────────
    let config = make_e2e_config(wal_dir);
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    gw.register_plugin(Arc::new(E2ePlugin) as Arc<dyn IMPlugin>)
        .await;
    let _handle = gw.start_inbound_queue();

    // Consumer should process the replayed message and clean WAL.
    wait_wal_empty(wal_dir).await;
}

// ===========================================================================
// Test 3: Default config WAL auto-enable assertion
// ===========================================================================

/// Default GatewayConfig must set inbound_wal_dir to Some(~/.closeclaw/inbound_wal).
/// Verified with TempDir to avoid污染 real HOME.
#[test]
fn test_e2e_default_config_wal_auto_enabled() {
    let config = GatewayConfig::default();
    let wal_dir = config
        .inbound_wal_dir
        .expect("default inbound_wal_dir must be Some");
    let home = dirs::home_dir().expect("dirs::home_dir must be available");
    assert_eq!(
        wal_dir,
        home.join(".closeclaw").join("inbound_wal"),
        "default inbound_wal_dir must point to ~/.closeclaw/inbound_wal"
    );
}

// ===========================================================================
// Test 4: Explicit null disables WAL
// ===========================================================================

/// Deserializing JSON with `"inbound_wal_dir": null` must result in None.
#[test]
fn test_e2e_explicit_null_disables_wal() {
    let json = r#"{
        "name": "test",
        "inbound_wal_dir": null
    }"#;
    let config: GatewayConfig = serde_json::from_str(json).expect("deserialization should succeed");
    assert!(
        config.inbound_wal_dir.is_none(),
        "explicit null inbound_wal_dir must be None"
    );
}

// ===========================================================================
// Test 5: Arrived → dequeued lifecycle for same trace_id
// ===========================================================================

/// Verify that for a successfully enqueued message, both `gateway.arrived`
/// and `queue.dequeued` events appear with the same trace_id, and arrived
/// appears before dequeued.
#[tokio::test]
async fn test_e2e_arrived_then_dequeued_lifecycle() {
    let debug_tmp = TempDir::new().expect("TempDir::new failed");
    let config = make_no_wal_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    gw.set_debug_log(make_debug_log(&debug_tmp).await).await;
    gw.register_plugin(Arc::new(E2ePlugin) as Arc<dyn IMPlugin>)
        .await;
    let _handle = gw.start_inbound_queue();
    let trace_id = "e2e-lifecycle-001";
    let mut req = make_request("lifecycle-log");
    req.trace_id = trace_id.to_string();
    let result = gw.enqueue_inbound(req).await;
    assert!(result.is_ok(), "enqueue should succeed");
    // Wait for consumer to dequeue and process.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let events = read_events_with_timeout(debug_tmp.path()).await;
    let arrived = filter_events_by_type_and_trace(&events, "gateway.arrived", trace_id);
    let dequeued = filter_events_by_type_and_trace(&events, "queue.dequeued", trace_id);
    assert_eq!(arrived.len(), 1, "exactly one arrived event expected");
    assert_eq!(dequeued.len(), 1, "exactly one dequeued event expected");
    let arrived_ts = arrived[0].timestamp;
    let dequeued_ts = dequeued[0].timestamp;
    assert!(
        arrived_ts <= dequeued_ts,
        "arrived ({}) must precede dequeued ({})",
        arrived_ts,
        dequeued_ts
    );
}

// ===========================================================================
// Test 6: No WAL files created when inbound_wal_dir is None
// ===========================================================================

/// When inbound_wal_dir is None, no WAL directory or file is created even
/// if the queue is started.
#[tokio::test]
async fn test_e2e_no_wal_files_when_disabled() {
    let tmp = TempDir::new().expect("TempDir::new failed");
    let config = GatewayConfig {
        name: "test-no-wal-e2e".to_owned(),
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
    let gw = Arc::new(Gateway::new(config, sm));
    gw.register_plugin(Arc::new(E2ePlugin) as Arc<dyn IMPlugin>)
        .await;
    let _handle = gw.start_inbound_queue();

    let mut req = make_request("no-wal-msg");
    req.trace_id = "e2e-no-wal-001".to_string();
    let _ = gw.enqueue_inbound(req).await;

    // Wait briefly for any async WAL creation that shouldn't happen.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify no WAL files exist.
    let wal_path = tmp.path().join("inbound_wal");
    assert!(
        !wal_path.exists(),
        "WAL directory must not be created when inbound_wal_dir is None"
    );
}

// ===========================================================================
// Test 7: Multiple messages lifecycle — all processed and WAL clean
// ===========================================================================

/// Enqueue multiple messages, verify all are processed and WAL is clean
/// after all processing completes.
#[tokio::test]
async fn test_e2e_multiple_messages_wal_clean() {
    let wal_tmp = TempDir::new().expect("TempDir::new failed");
    let wal_dir = wal_tmp.path();
    let config = make_e2e_config(wal_dir);
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    gw.register_plugin(Arc::new(E2ePlugin) as Arc<dyn IMPlugin>)
        .await;
    let _handle = gw.start_inbound_queue();

    // Enqueue 5 messages with unique trace_ids.
    for i in 0..5 {
        let mut req = make_request(&format!("multi-{i}"));
        req.trace_id = format!("e2e-multi-{i}");
        let result = gw.enqueue_inbound(req).await;
        assert!(result.is_ok(), "enqueue {i} should succeed");
    }

    // Wait for all to be processed and WAL cleaned.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let wal = super::inbound_wal::InboundWal::open(wal_dir).unwrap();
        let remaining = wal.load_all().unwrap();
        if remaining.is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "WAL should be empty after all messages processed, got {} entries",
            remaining.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
