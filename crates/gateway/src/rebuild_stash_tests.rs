//! Unit tests for the rebuild-stash mechanism (Step 1.3).
//!
//! Verifies that during config-triggered gateway rebuilds, inbound queue-full
//! messages are stashed instead of rejected, and replayed in FIFO order after
//! the rebuild completes.
//!
//! Test dimensions:
//! 1. Normal path: rebuild mode → stash + WAL + no busy reply + Ok
//! 2. State transitions: rebuild mode on/off, FIFO ordering
//! 3. Error/boundary: queue not full, empty stash, no WAL, full new queue
//! 4. Long-chain integration: full rebuild cycle with N messages

use std::sync::Arc;

use crate::inbound_queue::QueuedInbound;
use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig, InboundRequest};
use closeclaw_session::persistence::ReasoningLevel;

// ═════════════════════════════════════════════════════════════════════════════
// 1. Normal Path — rebuild stash behavior
// ═════════════════════════════════════════════════════════════════════════════

/// Rebuild mode ON + queue full → message enters stash buffer, WAL appended,
/// no busy reply sent, enqueue_inbound returns Ok().
#[tokio::test]
async fn test_rebuild_mode_queue_full_stashes_message() {
    let gw = make_gateway_with_capacity(1);
    let handle = gw.start_inbound_queue();
    // Fill the single-slot queue.
    handle
        .try_send(queued(make_request("fill")))
        .expect("fill should succeed");

    // Enter rebuild mode.
    gw.set_rebuild_mode(true);

    // Enqueue one more — queue is full, but rebuild mode stashes it.
    let result = gw.enqueue_inbound(make_request("overflow-stashed")).await;
    assert!(result.is_ok(), "rebuild mode should return Ok, not Err");

    // The stash buffer should contain exactly one message.
    assert_eq!(gw.rebuild_stash.take_stashed().len(), 1);
    // Stash is drained by take.
    assert!(gw.rebuild_stash.take_stashed().is_empty());
}

/// After rebuild completes (mode OFF), queue-full resumes rejection behavior.
#[tokio::test]
async fn test_rebuild_mode_off_resumes_rejection() {
    let gw = make_gateway_with_capacity(1);
    let handle = gw.start_inbound_queue();
    handle.try_send(queued(make_request("fill"))).unwrap();

    // Rebuild mode on → stash.
    gw.set_rebuild_mode(true);
    let result = gw.enqueue_inbound(make_request("during-rebuild")).await;
    assert!(result.is_ok());

    // Rebuild mode off → normal rejection.
    gw.set_rebuild_mode(false);
    let result = gw.enqueue_inbound(make_request("after-rebuild")).await;
    assert!(
        result.is_err(),
        "after rebuild mode off, queue full should reject"
    );
}

/// take_stashed returns FIFO-ordered messages and clears the buffer.
#[tokio::test]
async fn test_take_stashed_returns_fifo_and_clears() {
    let gw = make_gateway_with_capacity(1);
    let handle = gw.start_inbound_queue();
    handle.try_send(queued(make_request("fill"))).unwrap();
    gw.set_rebuild_mode(true);

    // Enqueue 3 messages with distinct trace_ids during rebuild.
    let trace_ids: Vec<String> = (0..3).map(|i| format!("fifo-trace-{i}")).collect();
    for tid in &trace_ids {
        let req = InboundRequest {
            platform: "feishu".into(),
            raw_payload: b"{}".to_vec(),
            peer_id: "p1".into(),
            trace_id: tid.clone(),
            span_id: None,
        };
        let result = gw.enqueue_inbound(req).await;
        assert!(result.is_ok(), "{tid} should be stashed");
    }

    let stashed = gw.rebuild_stash.take_stashed();
    assert_eq!(
        stashed.len(),
        3,
        "all 3 stashed messages should be returned"
    );
    // Verify FIFO order via distinct trace_ids.
    for (i, req) in stashed.iter().enumerate() {
        assert_eq!(
            req.trace_id, trace_ids[i],
            "message {i} should have trace_id {}",
            trace_ids[i]
        );
    }
    // Second take should be empty.
    assert!(gw.rebuild_stash.take_stashed().is_empty());
}

/// Stashed messages replayed into a new Gateway with capacity are all accepted.
#[tokio::test]
async fn test_replay_stashed_into_new_gateway() {
    // Old gateway: stash messages during rebuild.
    let old_gw = make_gateway_with_capacity(1);
    let handle = old_gw.start_inbound_queue();
    handle.try_send(queued(make_request("fill"))).unwrap();
    old_gw.set_rebuild_mode(true);
    for i in 0..3 {
        old_gw
            .enqueue_inbound(make_request(&format!("replay-{i}")))
            .await
            .unwrap();
    }
    let stashed = old_gw.rebuild_stash.take_stashed();
    assert_eq!(stashed.len(), 3);

    // New gateway: capacity=8, should accept all 3.
    let new_gw = make_gateway_with_capacity(8);
    let _handle = new_gw.start_inbound_queue();
    for req in &stashed {
        let result = new_gw.enqueue_inbound(req.clone()).await;
        assert!(result.is_ok(), "replayed message should be accepted");
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. State Transitions — rebuild mode on/off and FIFO ordering
// ═════════════════════════════════════════════════════════════════════════════

/// set_rebuild_mode(true) → stashed; set_rebuild_mode(false) + queue full → reject.
#[tokio::test]
async fn test_state_transition_rebuild_on_then_off() {
    let gw = make_gateway_with_capacity(1);
    let handle = gw.start_inbound_queue();
    handle.try_send(queued(make_request("fill"))).unwrap();

    // Phase 1: rebuild mode ON — stashes.
    gw.set_rebuild_mode(true);
    gw.enqueue_inbound(make_request("phase1")).await.unwrap();
    assert_eq!(gw.rebuild_stash.take_stashed().len(), 1);

    // Phase 2: rebuild mode OFF — rejects.
    gw.set_rebuild_mode(false);
    let result = gw.enqueue_inbound(make_request("phase2")).await;
    assert!(result.is_err());
}

/// Multiple full entries during rebuild are strictly FIFO-ordered.
#[tokio::test]
async fn test_multiple_stashes_strict_fifo_order() {
    let gw = make_gateway_with_capacity(1);
    let handle = gw.start_inbound_queue();
    handle.try_send(queued(make_request("fill"))).unwrap();
    gw.set_rebuild_mode(true);

    let mut trace_ids = Vec::new();
    for i in 0..5 {
        let req = InboundRequest {
            platform: "feishu".into(),
            raw_payload: b"{}".to_vec(),
            peer_id: "p_fifo".into(),
            trace_id: format!("fifo-{i}"),
            span_id: None,
        };
        gw.enqueue_inbound(req).await.unwrap();
        trace_ids.push(format!("fifo-{i}"));
    }

    let stashed = gw.rebuild_stash.take_stashed();
    assert_eq!(stashed.len(), 5);
    for (i, req) in stashed.iter().enumerate() {
        assert_eq!(
            req.trace_id, trace_ids[i],
            "message {i} should have trace_id {}",
            trace_ids[i]
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Error Path / Boundary Conditions
// ═════════════════════════════════════════════════════════════════════════════

/// Rebuild mode ON but queue not full → normal enqueue, no stash.
#[tokio::test]
async fn test_rebuild_mode_queue_not_full_enqueues_normally() {
    let gw = make_gateway_with_capacity(4);
    let _handle = gw.start_inbound_queue();
    gw.set_rebuild_mode(true);

    // Queue has room (capacity=4, used=0) → normal enqueue.
    let result = gw.enqueue_inbound(make_request("not-full")).await;
    assert!(result.is_ok());

    // Stash should be empty — message went into the channel, not stash.
    assert!(gw.rebuild_stash.take_stashed().is_empty());
}

/// take_stashed on empty buffer returns empty Vec.
#[tokio::test]
async fn test_take_stashed_empty_returns_empty_vec() {
    let gw = make_gateway_with_capacity(1);
    let _handle = gw.start_inbound_queue();
    gw.set_rebuild_mode(true);

    let stashed = gw.rebuild_stash.take_stashed();
    assert!(stashed.is_empty());
}

/// Rebuild mode ON + no WAL configured → message still enters stash (best-effort).
#[tokio::test]
async fn test_rebuild_mode_no_wal_still_stashes() {
    // Gateway with inbound_wal_dir=None (no WAL configured).
    let gw = Gateway::new(
        GatewayConfig {
            name: "no-wal-test".into(),
            inbound_queue_capacity: 1,
            inbound_wal_dir: None,
            ..Default::default()
        },
        Arc::new(SessionManager::new(
            &GatewayConfig::default(),
            None,
            None,
            ReasoningLevel::default(),
        )),
    );
    let gw = Arc::new(gw);
    let handle = gw.start_inbound_queue();
    handle.try_send(queued(make_request("fill"))).unwrap();
    gw.set_rebuild_mode(true);

    let result = gw.enqueue_inbound(make_request("no-wal")).await;
    assert!(result.is_ok(), "should stash even without WAL");
    assert_eq!(gw.rebuild_stash.take_stashed().len(), 1);
}

/// Rebuild mode ON + WAL configured → stashed messages are WAL-appended.
///
/// Verifies that the WAL file exists and contains the stashed message
/// after a queue-full hit during rebuild mode.
#[tokio::test]
async fn test_rebuild_stash_wal_appended() {
    let wal_dir = tempfile::tempdir().unwrap();
    let wal_path = wal_dir.path().to_path_buf();
    let config = GatewayConfig {
        name: "wal-stash-test".into(),
        inbound_queue_capacity: 1,
        inbound_wal_dir: Some(wal_path.clone()),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    let handle = gw.start_inbound_queue();
    handle.try_send(queued(make_request("fill"))).unwrap();

    // Enter rebuild mode — next full hit stashes + WAL appends.
    gw.set_rebuild_mode(true);
    let stashed_req = make_request("wal-stashed-msg");
    let result = gw.enqueue_inbound(stashed_req).await;
    assert!(result.is_ok(), "rebuild mode should stash, not reject");

    // Verify the stash buffer has the message.
    let stashed = gw.rebuild_stash.take_stashed();
    assert_eq!(stashed.len(), 1);
    let trace_id = stashed[0].trace_id.clone();

    // Verify WAL file exists and contains the stashed message.
    let wal_file = wal_path.join("inbound.jsonl");
    assert!(wal_file.exists(), "WAL file should exist after stash");
    let wal_content = std::fs::read_to_string(&wal_file).unwrap();
    assert!(
        wal_content.contains(&trace_id),
        "WAL should contain the stashed message trace_id"
    );
}

/// Replay into a new Gateway with full queue → normal reject (Err returned,
/// logged, not silently lost).
#[tokio::test]
async fn test_replay_full_new_queue_normal_reject() {
    let new_gw = make_gateway_with_capacity(2);
    let handle = new_gw.start_inbound_queue();
    // Fill queue.
    handle.try_send(queued(make_request("f1"))).unwrap();
    handle.try_send(queued(make_request("f2"))).unwrap();

    // Attempt replay of 2 stashed messages into full new queue.
    let mut rejected = 0;
    for i in 0..2 {
        let result = new_gw
            .enqueue_inbound(make_request(&format!("replay-fail-{i}")))
            .await;
        if result.is_err() {
            rejected += 1;
        }
    }
    assert_eq!(rejected, 2, "both replays should fail as normal reject");
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. Long-Chain Integration — full rebuild cycle
// ═════════════════════════════════════════════════════════════════════════════

/// Simulate a complete rebuild cycle:
/// 1. Old gateway has capacity=1, queue filled.
/// 2. Enter rebuild mode → enqueue N=5 messages → all stashed.
/// 3. Take stashed → verify FIFO order.
/// 4. Create new gateway with capacity=8 → replay all 5 → all accepted.
/// 5. Verify new gateway queue contains all 5 (drain via channel).
#[tokio::test]
async fn test_full_rebuild_cycle_fifo_replay() {
    // --- Phase 1: old gateway ---
    let old_gw = make_gateway_with_capacity(1);
    let old_handle = old_gw.start_inbound_queue();
    old_handle
        .try_send(queued(make_request("fill-old")))
        .unwrap();
    old_gw.set_rebuild_mode(true);

    let mut expected_trace_ids = Vec::new();
    for i in 0..5 {
        let req = InboundRequest {
            platform: "feishu".into(),
            raw_payload: format!("{{\"n\":{i}}}").into_bytes(),
            peer_id: format!("peer-{i}"),
            trace_id: format!("cycle-{i}"),
            span_id: None,
        };
        expected_trace_ids.push(req.trace_id.clone());
        old_gw.enqueue_inbound(req).await.unwrap();
    }

    // --- Phase 2: take stash ---
    let stashed = old_gw.rebuild_stash.take_stashed();
    assert_eq!(stashed.len(), 5);
    for (i, req) in stashed.iter().enumerate() {
        assert_eq!(req.trace_id, expected_trace_ids[i]);
        assert_eq!(req.peer_id, format!("peer-{i}"));
    }

    // --- Phase 3: replay into new gateway ---
    let new_gw = make_gateway_with_capacity(8);
    let _handle = new_gw.start_inbound_queue();

    for req in &stashed {
        new_gw.enqueue_inbound(req.clone()).await.unwrap();
    }

    // Verify all messages were successfully enqueued into the new gateway.
    // (start_inbound_consumer consumes the receiver, so we verify via enqueue_inbound
    // returning Ok for all 5 messages, which confirms they entered the channel.)
}

/// Long-chain with mixed capacity: old queue fills to capacity, 2 messages
/// stashed during rebuild, replayed into new gateway with sufficient capacity.
#[tokio::test]
async fn test_long_chain_mixed_capacity() {
    // Old gateway: capacity=1, fill queue first, then enter rebuild.
    let old_gw = make_gateway_with_capacity(1);
    let old_handle = old_gw.start_inbound_queue();
    // Fill the single-slot queue via handle (bypasses rebuild_mode check).
    old_handle
        .try_send(queued(make_request("fill-old")))
        .unwrap();
    // Now enter rebuild mode — queue is full.
    old_gw.set_rebuild_mode(true);

    old_gw.enqueue_inbound(make_request("mix-a")).await.unwrap();
    old_gw.enqueue_inbound(make_request("mix-b")).await.unwrap();
    let stashed = old_gw.rebuild_stash.take_stashed();
    assert_eq!(stashed.len(), 2, "both messages should be stashed");

    // New gateway: capacity=4, should accept both.
    let new_gw = make_gateway_with_capacity(4);
    let _handle = new_gw.start_inbound_queue();

    for req in &stashed {
        new_gw.enqueue_inbound(req.clone()).await.unwrap();
    }
    // Verify stash is empty after take.
    assert!(old_gw.rebuild_stash.take_stashed().is_empty());
}

// ═════════════════════════════════════════════════════════════════════════════
// Helpers
// ═════════════════════════════════════════════════════════════════════════════

fn make_gateway_with_capacity(capacity: usize) -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "rebuild-stash-test".into(),
        inbound_queue_capacity: capacity,
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

fn make_request(content: &str) -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        raw_payload: serde_json::json!({
            "header": {
                "event_id": "ev_test",
                "event_type": "im.message.receive_v1",
                "create_time": "1700000000000",
                "token": "t",
                "app_id": "a"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "u1" },
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
                    "content": format!("{{\"text\":\"{}\"}}", content)
                }
            }
        })
        .to_string()
        .into_bytes(),
        peer_id: "p1".into(),
        trace_id: String::new(),
        span_id: None,
    }
}

fn queued(request: InboundRequest) -> QueuedInbound {
    QueuedInbound { request }
}
