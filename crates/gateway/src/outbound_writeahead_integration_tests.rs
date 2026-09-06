//! Step 1.5 integration tests — outbound pending operations end-to-end.
//!
//! Covers the behavior dimensions not addressed by existing test files:
//!
//! - **Normal path end-to-end**: write-ahead → send Ok → ack-clear → no
//!   OutboundMessage op in final checkpoint.
//! - **Write-ahead persist failure**: checkpoint save fails before send →
//!   message NOT delivered (write-ahead is a hard gate per design doc).
//! - **Multiple outbound pending ops**: single session with multiple
//!   OutboundMessage ops, all delivered and cleared after drain.
//! - **State transition**: OutboundMessage + ToolCall + SubSessionSpawn
//!   coexist in pending_operations; drain only clears OutboundMessage.
//! - **Integration crash→drain→delivery**: write-ahead persisted, crash,
//!   restart, drain delivers via cache, then ack-clear removes op.
//! - **Cache mismatch**: cache entry without pending op → not drained.
//! - **Repeated drain**: idempotent re-drain does not re-deliver.
//! - **Stop/kill mixed ops**: all 3 op types coexist, only OutboundMessage
//!   processed by drain.

#[path = "outbound_writeahead_test_utils.rs"]
mod utils;
use crate::session_manager::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_common::IMPlugin;
use closeclaw_session::persistence::{PendingOperationType, PersistenceService, SessionCheckpoint};
use std::sync::Arc;
use tokio::sync::Mutex;
use utils::*;

// ═══════════════════════════════════════════════════════════════════════════
// 1. Normal path end-to-end: write-ahead → send Ok → ack-clear → no op
// ═══════════════════════════════════════════════════════════════════════════

/// Verify the complete normal-path lifecycle:
/// 1. write-ahead records OutboundMessage pending op
/// 2. plugin.send succeeds
/// 3. ack-clear removes the pending op
/// 4. Final checkpoint has NO OutboundMessage pending ops
/// 5. delivery record (outbound_pending) is persisted with sent=true
#[tokio::test]
async fn test_normal_path_writeahead_send_ackclear_no_op() {
    let persist = Arc::new(SnapshotMockPersist::new());
    let session_id = "normal-e2e";
    let (gw, entered, ok, texts) = setup_gw_with_persist(Arc::clone(&persist), session_id).await;
    let gw_arc = Arc::new(gw);
    let sid = session_id.to_string();
    let gw_clone = Arc::clone(&gw_arc);
    let handle = tokio::spawn(async move {
        gw_clone
            .send_outbound(&sid, "mock", "hello world", vec![], None, None)
            .await
    });

    // Phase 1: write-ahead persisted, send entered.
    entered.notified().await;
    {
        let snaps = persist.snapshots();
        assert_eq!(snaps.len(), 1, "write-ahead persist should have happened");
        assert_eq!(
            snaps[0].outbound_ops_count, 1,
            "write-ahead should record 1 OutboundMessage op"
        );
    }

    // Phase 2: let send complete → ack-clear → delivery record.
    ok.notify_one();
    let result: Result<_, _> = handle.await.expect("task should not panic");
    assert!(result.is_ok(), "send_outbound should succeed");

    // Verify the final checkpoint state.
    let cp = persist
        .checkpoints
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should exist");
    assert_eq!(
        outbound_ops_count(&cp),
        0,
        "ack-clear should remove all OutboundMessage pending ops"
    );
    assert_eq!(
        cp.outbound_pending.len(),
        1,
        "delivery record should be persisted"
    );
    assert!(
        cp.outbound_pending[0].sent,
        "delivery record should be marked sent"
    );

    // Verify the plugin received the message.
    let sent = texts.lock().await.clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0], "hello world");
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Write-ahead persist failure → message NOT sent (design doc aligned)
// ═══════════════════════════════════════════════════════════════════════════

/// When checkpoint save fails during write-ahead (before send), the
/// gateway MUST NOT proceed with plugin.send — per design doc:
/// "确认持久化成功后再执行实际操作". On write-ahead failure the message
/// is treated as a send failure (Notified path) and the op remains
/// for recovery retry.
#[tokio::test]
async fn test_writeahead_failure_message_not_sent() {
    // Fail on the 1st save (write-ahead).
    let persist = Arc::new(SnapshotMockPersist::fail_on_save(0));
    let session_id = "wa-fail";
    let sm = Arc::new(crate::SessionManager::new(
        &test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        closeclaw_session::persistence::ReasoningLevel::default(),
    ));
    register_session(&sm, session_id, "mock").await;
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    let gw = crate::Gateway::new(test_config(), Arc::clone(&sm)).with_checkpoint_manager(cm);

    let (plugin, entered, _ok, texts) = SyncPlugin::new();
    gw.register_plugin(Arc::new(plugin) as Arc<dyn IMPlugin>)
        .await;

    let gw_arc = Arc::new(gw);
    let sid = session_id.to_string();
    let gw_clone = Arc::clone(&gw_arc);
    let handle = tokio::spawn(async move {
        gw_clone
            .send_outbound(&sid, "mock", "should not be sent", vec![], None, None)
            .await
    });

    // Write-ahead fails → send should NOT be entered.
    // Give a short window for the task to settle without entering send.
    let result =
        tokio::time::timeout(std::time::Duration::from_millis(500), entered.notified()).await;
    assert!(
        result.is_err(),
        "plugin.send should NOT be entered when write-ahead fails"
    );

    let result: Result<_, _> = handle.await.expect("task should not panic");
    // send_outbound returns Ok (Notified path) — not an error, but not delivered.
    assert!(
        result.is_ok(),
        "send_outbound should return Ok (Notified) on write-ahead failure"
    );

    // Verify the message was NOT sent.
    let sent = texts.lock().await.clone();
    assert!(
        sent.is_empty(),
        "no message should be sent when write-ahead fails"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Multiple outbound pending ops — all delivered and cleared
// ═══════════════════════════════════════════════════════════════════════════

/// When a session has multiple OutboundMessage pending ops in its
/// checkpoint, drain_outbound_pending delivers all of them and clears
/// all ops from pending_operations.
#[tokio::test]
async fn test_multiple_outbound_pending_ops_all_cleared() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = test_config();
    let gw = crate::Gateway::new(gw_config, Arc::clone(&mgr));

    let plugin_texts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let texts_clone = Arc::clone(&plugin_texts);
    let plugin: Arc<dyn closeclaw_common::IMPlugin> =
        Arc::new(SimpleTrackPlugin { texts: texts_clone });
    gw.register_plugin(plugin).await;

    let gw_arc = Arc::new(gw);
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;

    let session_id = "multi-outbound";
    register_session(&mgr, session_id, "mock").await;

    // Build checkpoint with 3 OutboundMessage ops + matching cache entries.
    let mut cp = SessionCheckpoint::new(session_id.into());
    for i in 0..3 {
        let msg_id = format!("msg-{}", i);
        cp.record_outbound_pending_op(make_outbound_op(&msg_id, "mock"));
        cp.outbound_pending
            .push(closeclaw_common::PendingMessage::with_target_channel(
                msg_id,
                format!("content-{}", i),
                "mock".into(),
            ));
    }
    // Also add a ToolCall op (should NOT be cleared by drain).
    cp.pending_operations.push(make_tool_op("tool_1", "bash"));

    let persist = Arc::new(SnapshotMockPersist::new());
    persist
        .checkpoints
        .lock()
        .await
        .insert(session_id.to_string(), cp);
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    mgr.set_checkpoint_manager(cm).await;

    // Execute drain.
    let result: Result<usize, String> = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 3, "should deliver 3 messages");

    let sent = plugin_texts.lock().await.clone();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0], "content-0");
    assert_eq!(sent[1], "content-1");
    assert_eq!(sent[2], "content-2");

    // Verify: OutboundMessage ops cleared, ToolCall op preserved.
    let saved_cp = persist
        .checkpoints
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should exist");
    assert_eq!(
        outbound_ops_count(&saved_cp),
        0,
        "all OutboundMessage ops should be cleared"
    );
    let tool_ops: Vec<_> = saved_cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::ToolCall)
        .collect();
    assert_eq!(tool_ops.len(), 1, "ToolCall op should be preserved");
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. State transition: mixed op types — drain only clears OutboundMessage
// ═══════════════════════════════════════════════════════════════════════════

/// When pending_operations contains OutboundMessage + ToolCall +
/// SubSessionSpawn, drain only clears OutboundMessage ops while
/// preserving ToolCall and SubSessionSpawn ops for recovery notification.
#[tokio::test]
async fn test_mixed_op_types_drain_preserves_non_outbound() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = test_config();
    let gw = crate::Gateway::new(gw_config, Arc::clone(&mgr));

    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(SimpleTrackPlugin {
        texts: Arc::new(Mutex::new(Vec::new())),
    });
    gw.register_plugin(plugin).await;

    let gw_arc = Arc::new(gw);
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;

    let session_id = "mixed-ops-drain";
    register_session(&mgr, session_id, "mock").await;

    let mut cp = SessionCheckpoint::new(session_id.into());
    // OutboundMessage op — should be cleared by drain.
    cp.record_outbound_pending_op(make_outbound_op("out-msg", "mock"));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "out-msg".into(),
            "delivered content".into(),
            "mock".into(),
        ));
    // ToolCall op — should be preserved.
    cp.pending_operations.push(make_tool_op("tool_1", "bash"));
    // SubSessionSpawn op — should be preserved.
    cp.pending_operations.push(make_child_op("child_1", "eda"));

    let persist = Arc::new(SnapshotMockPersist::new());
    persist
        .checkpoints
        .lock()
        .await
        .insert(session_id.to_string(), cp);
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    mgr.set_checkpoint_manager(cm).await;

    let result: Result<usize, String> = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 1, "should deliver 1 outbound message");

    // Verify only OutboundMessage op was cleared; others preserved.
    let saved_cp = persist
        .checkpoints
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should exist");
    let op_types: Vec<&PendingOperationType> = saved_cp
        .pending_operations
        .iter()
        .map(|op| &op.op_type)
        .collect();
    assert!(
        !op_types.contains(&&PendingOperationType::OutboundMessage),
        "OutboundMessage should be cleared"
    );
    assert!(
        op_types.contains(&&PendingOperationType::ToolCall),
        "ToolCall should be preserved"
    );
    assert!(
        op_types.contains(&&PendingOperationType::SubSessionSpawn),
        "SubSessionSpawn should be preserved"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Integration: crash → restart → drain delivers via cache
// ═══════════════════════════════════════════════════════════════════════════

/// Full integration scenario:
/// 1. Write-ahead records OutboundMessage op in checkpoint.
/// 2. Crash before send (task aborted).
/// 3. "Restart": load checkpoint from persistence.
/// 4. Drain delivers message via outbound_pending cache.
/// 5. Op is cleared from pending_operations.
///
/// This verifies that a message is NOT lost after crash.
#[tokio::test]
async fn test_crash_restart_drain_delivers_message() {
    clear_global_prompt_state();

    // Phase 1: simulate write-ahead by pre-populating checkpoint
    // with an OutboundMessage pending op + outbound_pending cache entry.
    let session_id = "crash-restart";
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op("crash-msg", "mock"));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "crash-msg".into(),
            "saved before crash".into(),
            "mock".into(),
        ));
    // Also have a ToolCall op (should survive alongside).
    cp.pending_operations.push(make_tool_op("tool_1", "exec"));

    let persist = Arc::new(SnapshotMockPersist::new());
    persist
        .checkpoints
        .lock()
        .await
        .insert(session_id.to_string(), cp);

    // Phase 2: "restart" — set up SessionManager with the persisted checkpoint.
    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = test_config();
    let gw = crate::Gateway::new(gw_config, Arc::clone(&mgr));

    let plugin_texts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let texts_clone = Arc::clone(&plugin_texts);
    let plugin: Arc<dyn closeclaw_common::IMPlugin> =
        Arc::new(SimpleTrackPlugin { texts: texts_clone });
    gw.register_plugin(plugin).await;

    let gw_arc = Arc::new(gw);
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;
    register_session(&mgr, session_id, "mock").await;
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    mgr.set_checkpoint_manager(cm).await;

    // Phase 3: drain delivers the message.
    let result: Result<usize, String> = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(
        result.unwrap(),
        1,
        "should deliver 1 message that survived the crash"
    );

    let sent = plugin_texts.lock().await.clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0], "saved before crash");

    // Verify op cleared, ToolCall preserved.
    let saved_cp = persist
        .checkpoints
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should exist");
    assert_eq!(
        outbound_ops_count(&saved_cp),
        0,
        "OutboundMessage op should be cleared after drain"
    );
    let tool_ops: Vec<_> = saved_cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::ToolCall)
        .collect();
    assert_eq!(tool_ops.len(), 1, "ToolCall op should be preserved");
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. CheckpointManager integrates with write-ahead/ack-clear
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that write-ahead and ack-clear through the CheckpointManager
/// correctly persist and clear OutboundMessage pending ops.
#[tokio::test]
async fn test_checkpoint_manager_writeahead_ackclear_roundtrip() {
    use closeclaw_session::checkpoint_manager::CheckpointManager;

    let persist = Arc::new(SnapshotMockPersist::new());
    let cm = CheckpointManager::new(Arc::clone(&persist) as Arc<dyn PersistenceService>);

    // Create initial checkpoint.
    let cp = SessionCheckpoint::new("cm-test".into());
    cm.save(cp).await.unwrap();

    // Load and add a write-ahead op.
    let mut loaded = cm.load("cm-test").await.unwrap().unwrap();
    loaded.record_outbound_pending_op(make_outbound_op("wa-msg", "mock"));
    cm.save(loaded).await.unwrap();

    // Verify op exists.
    let check = cm.load("cm-test").await.unwrap().unwrap();
    assert_eq!(outbound_ops_count(&check), 1, "write-ahead op should exist");
    assert_eq!(check.pending_operations[0].op_id, "wa-msg");

    // Load and ack-clear.
    let mut loaded = cm.load("cm-test").await.unwrap().unwrap();
    loaded.clear_outbound_pending_op("wa-msg");
    cm.save(loaded).await.unwrap();

    // Verify op cleared.
    let check = cm.load("cm-test").await.unwrap().unwrap();
    assert_eq!(
        outbound_ops_count(&check),
        0,
        "ack-clear should remove the OutboundMessage op"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. Cache mismatch: cache entry without pending op → not drained
// ═══════════════════════════════════════════════════════════════════════════

/// When outbound_pending cache has entries but there are no
/// corresponding OutboundMessage pending ops, drain returns 0.
/// Cache entries without ops are not delivery candidates.
#[tokio::test]
async fn test_cache_entry_without_pending_op_not_drained() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = test_config();
    let gw = crate::Gateway::new(gw_config, Arc::clone(&mgr));

    let plugin_texts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let texts_clone = Arc::clone(&plugin_texts);
    let plugin: Arc<dyn closeclaw_common::IMPlugin> =
        Arc::new(SimpleTrackPlugin { texts: texts_clone });
    gw.register_plugin(plugin).await;

    let gw_arc = Arc::new(gw);
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;

    let session_id = "cache-no-op";
    register_session(&mgr, session_id, "mock").await;

    // Checkpoint: outbound_pending cache has entries, but NO OutboundMessage ops.
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "cache-msg-1".into(),
            "orphan cache content".into(),
            "mock".into(),
        ));
    // Add a ToolCall op (should NOT trigger drain).
    cp.pending_operations.push(make_tool_op("tool_1", "bash"));

    let persist = Arc::new(SnapshotMockPersist::new());
    persist
        .checkpoints
        .lock()
        .await
        .insert(session_id.to_string(), cp);
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    mgr.set_checkpoint_manager(cm).await;

    let result: Result<usize, String> = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(
        result.unwrap(),
        0,
        "no OutboundMessage ops → nothing delivered"
    );

    let sent = plugin_texts.lock().await.clone();
    assert!(sent.is_empty(), "plugin should not receive any messages");
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. Repeated restart drain: same op drained multiple times
// ═══════════════════════════════════════════════════════════════════════════

/// After a successful drain that clears the op, a second drain call
/// returns 0 — verifying idempotent re-drain does not re-deliver.
#[tokio::test]
async fn test_repeated_drain_idempotent() {
    clear_global_prompt_state();

    let session_id = "repeat-drain";
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op("repeat-msg", "mock"));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "repeat-msg".into(),
            "repeat content".into(),
            "mock".into(),
        ));

    let persist = Arc::new(SnapshotMockPersist::new());
    persist
        .checkpoints
        .lock()
        .await
        .insert(session_id.to_string(), cp);

    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = test_config();
    let gw = crate::Gateway::new(gw_config, Arc::clone(&mgr));

    let plugin_texts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let texts_clone = Arc::clone(&plugin_texts);
    let plugin: Arc<dyn closeclaw_common::IMPlugin> =
        Arc::new(SimpleTrackPlugin { texts: texts_clone });
    gw.register_plugin(plugin).await;

    let gw_arc = Arc::new(gw);
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;
    register_session(&mgr, session_id, "mock").await;
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    mgr.set_checkpoint_manager(cm).await;

    // First drain: delivers the message.
    let result1 = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(
        result1.is_ok(),
        "first drain should succeed: {:?}",
        result1.err()
    );
    assert_eq!(result1.unwrap(), 1, "first drain should deliver 1 message");

    // Second drain: op already cleared → 0 delivered.
    let result2 = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(
        result2.is_ok(),
        "second drain should succeed: {:?}",
        result2.err()
    );
    assert_eq!(
        result2.unwrap(),
        0,
        "second drain should deliver 0 (op already cleared)"
    );

    // Plugin only received 1 message total (not duplicated).
    let sent = plugin_texts.lock().await.clone();
    assert_eq!(sent.len(), 1, "message should be delivered exactly once");
}

// ═══════════════════════════════════════════════════════════════════════════
// 9. Stop/kill state transition: all 3 op types coexist
// ═══════════════════════════════════════════════════════════════════════════

/// During stop/kill timing, pending_operations may contain all three
/// op types simultaneously. On restart, drain only processes
/// OutboundMessage ops while ToolCall and SubSessionSpawn are
/// preserved for their respective recovery paths.
#[tokio::test]
async fn test_stop_kill_mixed_ops_drain_only_outbound() {
    clear_global_prompt_state();

    let session_id = "stop-kill-mixed";
    let mut cp = SessionCheckpoint::new(session_id.into());
    // OutboundMessage ops — should be drained.
    cp.record_outbound_pending_op(make_outbound_op("out-1", "mock"));
    cp.record_outbound_pending_op(make_outbound_op("out-2", "mock"));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "out-1".into(),
            "outbound content 1".into(),
            "mock".into(),
        ));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "out-2".into(),
            "outbound content 2".into(),
            "mock".into(),
        ));
    // ToolCall op — should be preserved (not processed by drain).
    cp.pending_operations
        .push(make_tool_op("tool_stuck", "bash"));
    // SubSessionSpawn op — should be preserved.
    cp.pending_operations
        .push(make_child_op("child_stuck", "eda"));

    let persist = Arc::new(SnapshotMockPersist::new());
    persist
        .checkpoints
        .lock()
        .await
        .insert(session_id.to_string(), cp);

    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = test_config();
    let gw = crate::Gateway::new(gw_config, Arc::clone(&mgr));

    let plugin_texts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let texts_clone = Arc::clone(&plugin_texts);
    let plugin: Arc<dyn closeclaw_common::IMPlugin> =
        Arc::new(SimpleTrackPlugin { texts: texts_clone });
    gw.register_plugin(plugin).await;

    let gw_arc = Arc::new(gw);
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;
    register_session(&mgr, session_id, "mock").await;
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    mgr.set_checkpoint_manager(cm).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 2, "should deliver 2 outbound messages");

    let sent = plugin_texts.lock().await.clone();
    assert_eq!(sent.len(), 2);
    assert!(sent.contains(&"outbound content 1".to_string()));
    assert!(sent.contains(&"outbound content 2".to_string()));

    // Verify: OutboundMessage ops cleared, ToolCall + SubSessionSpawn preserved.
    let saved_cp = persist
        .checkpoints
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should exist");
    assert_eq!(
        outbound_ops_count(&saved_cp),
        0,
        "OutboundMessage ops should be cleared"
    );
    let op_types: Vec<&PendingOperationType> = saved_cp
        .pending_operations
        .iter()
        .map(|op| &op.op_type)
        .collect();
    assert!(
        op_types.contains(&&PendingOperationType::ToolCall),
        "ToolCall should be preserved"
    );
    assert!(
        op_types.contains(&&PendingOperationType::SubSessionSpawn),
        "SubSessionSpawn should be preserved"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 10. Send failure → op remains → drain retry succeeds
// ═══════════════════════════════════════════════════════════════════════════

/// End-to-end error path:
/// 1. send_outbound fails → pending op remains
/// 2. drain_outbound_pending_for_session retries → succeeds
/// 3. Op is cleared
///
/// Verifies that a message is NOT lost after send failure + restart.
#[tokio::test]
async fn test_send_failure_op_remains_then_drain_succeeds() {
    clear_global_prompt_state();

    // Phase 1: simulate failed send — write-ahead op exists, outbound_pending
    // cache entry exists but send was never completed.
    let session_id = "fail-then-retry";
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op("retry-msg", "mock"));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "retry-msg".into(),
            "retry content".into(),
            "mock".into(),
        ));

    let persist = Arc::new(SnapshotMockPersist::new());
    persist
        .checkpoints
        .lock()
        .await
        .insert(session_id.to_string(), cp);

    // Phase 2: "restart" and drain with a working plugin.
    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = test_config();
    let gw = crate::Gateway::new(gw_config, Arc::clone(&mgr));

    let plugin_texts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let texts_clone = Arc::clone(&plugin_texts);
    let plugin: Arc<dyn closeclaw_common::IMPlugin> =
        Arc::new(SimpleTrackPlugin { texts: texts_clone });
    gw.register_plugin(plugin).await;

    let gw_arc = Arc::new(gw);
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;
    register_session(&mgr, session_id, "mock").await;
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    mgr.set_checkpoint_manager(cm).await;

    // Drain should deliver the message that was stuck after send failure.
    let result: Result<usize, String> = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 1, "should deliver 1 message on retry");

    let sent = plugin_texts.lock().await.clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0], "retry content");

    // Verify op cleared.
    let saved_cp = persist
        .checkpoints
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should exist");
    assert_eq!(
        outbound_ops_count(&saved_cp),
        0,
        "OutboundMessage op should be cleared after successful drain"
    );
}
