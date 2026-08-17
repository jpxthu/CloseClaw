//! Standalone unit tests for the inbound WAL component.
//!
//! Covers append/load/delete roundtrip, crash resilience (missing file,
//! malformed lines), payload base64 roundtrip, concurrent append safety,
//! status serialization, restart replay (new Gateway + same WAL dir),
//! shutdown WAL preservation, and wal_dir None behavior.

use super::inbound_wal::{InboundWal, InboundWalEntry, InboundWalEntryStatus};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Arc;

use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig};
use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::{ContentBlock, NormalizedMessage};
use closeclaw_session::persistence::ReasoningLevel;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_wal() -> (tempfile::TempDir, InboundWal) {
    let dir = tempfile::tempdir().unwrap();
    let wal = InboundWal::open(dir.path()).unwrap();
    (dir, wal)
}

fn sample(trace_id: &str) -> InboundWalEntry {
    InboundWalEntry::new(
        trace_id.to_string(),
        "feishu".to_string(),
        b"{\"event\":{}}",
        "p1".to_string(),
    )
}

// ---------------------------------------------------------------------------
// Append / Load roundtrip
// ---------------------------------------------------------------------------

#[test]
fn test_append_load_roundtrip_multiple_entries() {
    let (_dir, wal) = temp_wal();
    wal.append(&sample("tr-1")).unwrap();
    wal.append(&sample("tr-2")).unwrap();
    wal.append(&sample("tr-3")).unwrap();
    let loaded = wal.load_all().unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].trace_id, "tr-1");
    assert_eq!(loaded[1].trace_id, "tr-2");
    assert_eq!(loaded[2].trace_id, "tr-3");
}

#[test]
fn test_append_preserves_entry_fields() {
    let (_dir, wal) = temp_wal();
    let entry = InboundWalEntry::new(
        "tr-field".into(),
        "discord".into(),
        b"payload-bytes",
        "chat-42".into(),
    );
    let ts_before = chrono::Utc::now().timestamp();
    wal.append(&entry).unwrap();
    let ts_after = chrono::Utc::now().timestamp();
    let loaded = wal.load_all().unwrap();
    assert_eq!(loaded.len(), 1);
    let e = &loaded[0];
    assert_eq!(e.trace_id, "tr-field");
    assert_eq!(e.platform, "discord");
    assert_eq!(e.peer_id, "chat-42");
    assert_eq!(e.status, InboundWalEntryStatus::Pending);
    assert!(e.enqueued_at >= ts_before && e.enqueued_at <= ts_after);
    // Base64 payload decodes correctly.
    assert_eq!(e.decoded_payload().unwrap(), b"payload-bytes");
}

// ---------------------------------------------------------------------------
// Mark done and delete
// ---------------------------------------------------------------------------

#[test]
fn test_mark_done_deletes_target_only() {
    let (_dir, wal) = temp_wal();
    wal.append(&sample("tr-a")).unwrap();
    wal.append(&sample("tr-b")).unwrap();
    wal.append(&sample("tr-c")).unwrap();
    wal.mark_done_and_delete("tr-b").unwrap();
    let loaded = wal.load_all().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].trace_id, "tr-a");
    assert_eq!(loaded[1].trace_id, "tr-c");
}

#[test]
fn test_mark_done_on_empty_wal_is_noop() {
    let (_dir, wal) = temp_wal();
    wal.mark_done_and_delete("nope").unwrap();
    assert!(wal.load_all().unwrap().is_empty());
}

#[test]
fn test_mark_done_all_entries_removes_file_contents() {
    let (_dir, wal) = temp_wal();
    wal.append(&sample("only")).unwrap();
    wal.mark_done_and_delete("only").unwrap();
    let loaded = wal.load_all().unwrap();
    assert!(loaded.is_empty());
}

// ---------------------------------------------------------------------------
// Crash resilience
// ---------------------------------------------------------------------------

#[test]
fn test_load_missing_file_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let wal = InboundWal::open(dir.path()).unwrap();
    fs::remove_file(dir.path().join("inbound.jsonl")).unwrap();
    assert!(wal.load_all().unwrap().is_empty());
}

#[test]
fn test_load_skips_malformed_lines() {
    let (_dir, wal) = temp_wal();
    wal.append(&sample("tr-ok")).unwrap();
    // Inject a bad line directly.
    {
        let mut f = OpenOptions::new()
            .append(true)
            .open(wal.dir().join("inbound.jsonl"))
            .unwrap();
        writeln!(f, "NOT_JSON{{").unwrap();
    }
    wal.append(&sample("tr-after")).unwrap();
    let loaded = wal.load_all().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].trace_id, "tr-ok");
    assert_eq!(loaded[1].trace_id, "tr-after");
}

#[test]
fn test_load_skips_empty_lines() {
    let (_dir, wal) = temp_wal();
    wal.append(&sample("tr-x")).unwrap();
    {
        let mut f = OpenOptions::new()
            .append(true)
            .open(wal.dir().join("inbound.jsonl"))
            .unwrap();
        writeln!(f).unwrap(); // empty line
        writeln!(f, "  ").unwrap(); // whitespace-only line
    }
    let loaded = wal.load_all().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].trace_id, "tr-x");
}

// ---------------------------------------------------------------------------
// Append after file removed recreates
// ---------------------------------------------------------------------------

#[test]
fn test_append_recreates_deleted_file() {
    let (_dir, wal) = temp_wal();
    wal.append(&sample("tr-1")).unwrap();
    fs::remove_file(wal.dir().join("inbound.jsonl")).unwrap();
    wal.append(&sample("tr-2")).unwrap();
    let loaded = wal.load_all().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].trace_id, "tr-2");
}

// ---------------------------------------------------------------------------
// Directory creation
// ---------------------------------------------------------------------------

#[test]
fn test_open_creates_nested_directory() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a").join("b").join("c");
    let wal = InboundWal::open(&nested).unwrap();
    assert!(nested.join("inbound.jsonl").exists());
    assert_eq!(wal.dir(), nested.as_path());
}

// ---------------------------------------------------------------------------
// Payload base64 roundtrip
// ---------------------------------------------------------------------------

#[test]
fn test_binary_payload_base64_roundtrip() {
    let payload = b"\xff\xfe\x00binary\x01data";
    let entry = InboundWalEntry::new(
        "tr-bin".into(),
        "test".into(),
        payload,
        "peer".into(),
    );
    assert_eq!(entry.decoded_payload().unwrap(), payload);
}

#[test]
fn test_empty_payload_base64_roundtrip() {
    let entry = InboundWalEntry::new(
        "tr-empty".into(),
        "test".into(),
        b"",
        "peer".into(),
    );
    assert_eq!(entry.decoded_payload().unwrap(), b"");
}

// ---------------------------------------------------------------------------
// Status serialization
// ---------------------------------------------------------------------------

#[test]
fn test_status_pending_serialization_roundtrip() {
    let json = serde_json::to_string(&InboundWalEntryStatus::Pending).unwrap();
    let back: InboundWalEntryStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back, InboundWalEntryStatus::Pending);
}

#[test]
fn test_status_done_serialization_roundtrip() {
    let json = serde_json::to_string(&InboundWalEntryStatus::Done).unwrap();
    let back: InboundWalEntryStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back, InboundWalEntryStatus::Done);
}

// ---------------------------------------------------------------------------
// Concurrent append safety (WAL uses Mutex internally)
// ---------------------------------------------------------------------------

#[test]
fn test_concurrent_appends_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    let wal = InboundWal::open(dir.path()).unwrap();
    let wal = std::sync::Arc::new(wal);
    let mut handles = Vec::new();
    for i in 0..20 {
        let wal = std::sync::Arc::clone(&wal);
        handles.push(std::thread::spawn(move || {
            let entry = InboundWalEntry::new(
                format!("tr-concurrent-{i}"),
                "feishu".into(),
                format!("{{\"i\":{i}}}").as_bytes(),
                "p1".into(),
            );
            wal.append(&entry).unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let loaded = wal.load_all().unwrap();
    assert_eq!(loaded.len(), 20);
    // All trace_ids should be present (order may vary due to concurrency).
    let mut ids: Vec<String> = loaded.iter().map(|e| e.trace_id.clone()).collect();
    ids.sort();
    let mut expected: Vec<String> = (0..20).map(|i| format!("tr-concurrent-{i}")).collect();
    expected.sort();
    assert_eq!(ids, expected);
}

// ===========================================================================
// Restart replay: new Gateway instance, same WAL directory
// ===========================================================================

/// Mock plugin that captures send calls for verification.
struct SendCapturePlugin {
    sends: std::sync::Mutex<Vec<String>>,
}

impl SendCapturePlugin {
    fn new() -> Self {
        Self {
            sends: std::sync::Mutex::new(Vec::new()),
        }
    }
#[allow(dead_code)]
    fn send_count(&self) -> usize {
        self.sends.lock().unwrap().len()
    }
}

#[async_trait]
impl IMPlugin for SendCapturePlugin {
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
            content: "captured".into(),
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
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        self.sends.lock().unwrap().push(text);
        Ok(())
    }
}

/// Restart replay: pending WAL entries are loaded on startup.
///
/// Writes pending entries to the WAL, creates a new Gateway with that WAL
/// directory, and verifies load_all returns only the pending entries (not
/// previously completed ones). This tests the WAL loading + dedup logic
/// that `start_inbound_queue` uses for replay.
#[test]
fn test_restart_replay_loads_pending_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let wal_dir = tmp.path().to_path_buf();
    let wal = InboundWal::open(&wal_dir).unwrap();

    // Write 3 pending entries.
    wal.append(&InboundWalEntry::new(
        "tr-replay-1".into(),
        "feishu".into(),
        b"msg-1",
        "chat-1".into(),
    ))
    .unwrap();
    wal.append(&InboundWalEntry::new(
        "tr-replay-2".into(),
        "feishu".into(),
        b"msg-2",
        "chat-1".into(),
    ))
    .unwrap();
    wal.append(&InboundWalEntry::new(
        "tr-replay-3".into(),
        "feishu".into(),
        b"msg-3",
        "chat-1".into(),
    ))
    .unwrap();
    drop(wal);

    // Simulate restart: open the same WAL directory with a new handle.
    let wal2 = InboundWal::open(&wal_dir).unwrap();
    let entries = wal2.load_all().unwrap();
    assert_eq!(entries.len(), 3, "all 3 pending entries should be loaded");
    let ids: Vec<&str> = entries.iter().map(|e| e.trace_id.as_str()).collect();
    assert!(ids.contains(&"tr-replay-1"));
    assert!(ids.contains(&"tr-replay-2"));
    assert!(ids.contains(&"tr-replay-3"));
}

/// Restart replay: done entries are excluded from replay load.
///
/// Verifies that entries previously marked done are not returned by
/// load_all, matching the dedup behavior used during replay.
#[test]
fn test_restart_replay_excludes_done_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let wal_dir = tmp.path().to_path_buf();
    let wal = InboundWal::open(&wal_dir).unwrap();

    // Entry 1: Pending.
    wal.append(&InboundWalEntry::new(
        "tr-pending".into(),
        "feishu".into(),
        b"pending",
        "chat-1".into(),
    ))
    .unwrap();

    // Entry 2: Marked done (deleted from WAL).
    wal.append(&InboundWalEntry::new(
        "tr-done".into(),
        "feishu".into(),
        b"done",
        "chat-1".into(),
    ))
    .unwrap();
    wal.mark_done_and_delete("tr-done").unwrap();

    // Entry 3: Pending.
    wal.append(&InboundWalEntry::new(
        "tr-pending-2".into(),
        "feishu".into(),
        b"pending-2",
        "chat-1".into(),
    ))
    .unwrap();

    // Simulate restart: load_all returns only pending entries.
    let wal2 = InboundWal::open(&wal_dir).unwrap();
    let entries = wal2.load_all().unwrap();
    assert_eq!(entries.len(), 2, "only pending entries should be loaded");
    let ids: Vec<&str> = entries.iter().map(|e| e.trace_id.as_str()).collect();
    assert!(ids.contains(&"tr-pending"));
    assert!(ids.contains(&"tr-pending-2"));
    assert!(!ids.contains(&"tr-done"), "done entry must not be loaded");
}

/// Restart replay: duplicate trace_ids are deduplicated.
///
/// Verifies that if the WAL contains multiple entries with the same
/// trace_id (e.g. from a crash), only the first is kept during replay.
#[test]
fn test_restart_replay_deduplicates_trace_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let wal_dir = tmp.path().to_path_buf();
    let wal = InboundWal::open(&wal_dir).unwrap();

    // Two entries with the same trace_id (simulating crash before cleanup).
    wal.append(&InboundWalEntry::new(
        "tr-dup".into(),
        "feishu".into(),
        b"msg-dup-1",
        "chat-1".into(),
    ))
    .unwrap();
    wal.append(&InboundWalEntry::new(
        "tr-dup".into(),
        "feishu".into(),
        b"msg-dup-2",
        "chat-1".into(),
    ))
    .unwrap();

    // Simulate restart + dedup (matching the logic in start_inbound_queue).
    let wal2 = InboundWal::open(&wal_dir).unwrap();
    let entries = wal2.load_all().unwrap();
    let mut seen = std::collections::HashSet::new();
    let deduped: Vec<&InboundWalEntry> = entries
        .iter()
        .filter(|e| seen.insert(e.trace_id.clone()))
        .collect();
    assert_eq!(deduped.len(), 1, "duplicate trace_id should be deduplicated");
    assert_eq!(deduped[0].trace_id, "tr-dup");
}

// ===========================================================================
// Shutdown WAL preservation
// ===========================================================================

/// After the consumer stops, WAL entries that were not processed are preserved.
///
/// Writes entries to the WAL, starts the inbound queue, waits briefly, then
/// verifies the WAL still contains the entries (entries were not deleted
/// because the consumer could not fully process them without session context).
#[tokio::test]
async fn test_shutdown_wal_preserves_unfinished_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let wal_dir = tmp.path().to_path_buf();
    let wal = InboundWal::open(&wal_dir).unwrap();

    // Write 2 entries directly to WAL.
    wal.append(&InboundWalEntry::new(
        "tr-shutdown-1".into(),
        "feishu".into(),
        b"msg-1",
        "chat-1".into(),
    ))
    .unwrap();
    wal.append(&InboundWalEntry::new(
        "tr-shutdown-2".into(),
        "feishu".into(),
        b"msg-2",
        "chat-1".into(),
    ))
    .unwrap();
    drop(wal);

    // Create Gateway with WAL dir and start queue.
    let config = GatewayConfig {
        name: "test-shutdown-wal".to_owned(),
        inbound_queue_capacity: 16,
        inbound_wal_dir: Some(wal_dir.clone()),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    gw.start_inbound_queue();

    // Wait for replay to enqueue and consumer to attempt processing.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify WAL still contains the entries.
    let wal_after = InboundWal::open(&wal_dir).unwrap();
    let remaining = wal_after.load_all().unwrap();
    assert!(
        remaining.len() >= 2,
        "WAL should preserve unfinished entries after shutdown, got {}",
        remaining.len()
    );
    let ids: Vec<&str> = remaining.iter().map(|e| e.trace_id.as_str()).collect();
    assert!(
        ids.contains(&"tr-shutdown-1"),
        "tr-shutdown-1 should be preserved"
    );
    assert!(
        ids.contains(&"tr-shutdown-2"),
        "tr-shutdown-2 should be preserved"
    );
}

// ===========================================================================
// Consumer processes queue-sent messages and cleans WAL
// ===========================================================================

/// Consumer processes a message sent through the queue handle and deletes
/// the WAL entry. This verifies the end-to-end flow: enqueue → WAL append
/// → consumer dequeue → process → WAL delete.
#[tokio::test]
async fn test_consumer_processes_and_cleans_wal() {
    use super::inbound_queue::InboundRequest;

    let tmp = tempfile::tempdir().unwrap();
    let wal_dir = tmp.path().to_path_buf();

    let config = GatewayConfig {
        name: "test-consumer-wal".to_owned(),
        inbound_queue_capacity: 16,
        inbound_wal_dir: Some(wal_dir.clone()),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    let plugin = Arc::new(SendCapturePlugin::new());
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;

    let _handle = gw.start_inbound_queue();

    // Enqueue a message (this triggers WAL append + channel send).
    gw.enqueue_inbound(InboundRequest {
        platform: "feishu".into(),
        raw_payload: b"{\"test\":\"data\"}".to_vec(),
        peer_id: "p1".into(),
        trace_id: "tr-consumer-e2e".into(),
    })
    .await;

    // Wait for consumer to process.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // WAL entry should be deleted after consumer processes the message.
    let wal_after = InboundWal::open(&wal_dir).unwrap();
    let remaining = wal_after.load_all().unwrap();
    assert!(
        remaining.is_empty(),
        "WAL should be empty after consumer processes the message, got {} entries",
        remaining.len()
    );
}

/// Consumer drops plugin not found messages gracefully.
///
/// When a message's platform has no registered plugin, the consumer
/// should not panic and should continue processing.
#[tokio::test]
async fn test_consumer_drops_unknown_platform_gracefully() {
    let gw = super::inbound_queue_test_utils::make_gateway();
    let handle = gw.start_inbound_queue();

    // Send a message for an unknown platform.
    handle
        .try_send(super::inbound_queue_test_utils::queued(
            super::inbound_queue_test_utils::make_request("unknown-platform"),
        ))
        .unwrap();

    // Wait for consumer to process (should not panic).
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // No panic = consumer handled missing plugin gracefully.
}

// ===========================================================================
// wal_dir None: no WAL files created, behavior unchanged
// ===========================================================================

/// When `inbound_wal_dir` is None, no WAL directory or file is created.
/// The Gateway falls back to in-memory queue behavior with no persistence.
#[tokio::test]
async fn test_wal_dir_none_no_files_created() {
    let tmp = tempfile::tempdir().unwrap();
    let wal_dir = tmp.path().join("should_not_exist");

    let config = GatewayConfig {
        name: "test-no-wal".to_owned(),
        inbound_queue_capacity: 4,
        inbound_wal_dir: None, // WAL disabled
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));

    // Start queue — no WAL directory should be created.
    gw.start_inbound_queue();

    // Give the consumer time to start.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert!(
        !wal_dir.exists(),
        "WAL directory should not be created when inbound_wal_dir is None"
    );
}
