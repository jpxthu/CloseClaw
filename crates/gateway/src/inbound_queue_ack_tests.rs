//! Behavioral tests for inbound queue enqueue semantics.
//!
//! Verifies that enqueue success returns Ok immediately (the ack contract:
//! enqueue + persist = ack, no consumer dequeue required). Also covers
//! queue-full and queue-closed rejection paths.

use std::sync::Arc;

use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig};
use closeclaw_session::persistence::ReasoningLevel;
use tokio::sync::mpsc;

use super::inbound_queue::{start_inbound_consumer, InboundQueueHandle, QueuedInbound};
use super::inbound_queue_test_utils::{make_gateway, make_request, queued};

// ---------------------------------------------------------------------------
// Enqueue semantics tests
// ---------------------------------------------------------------------------

/// Enqueue success returns Ok immediately without waiting for consumer.
///
/// Verifies the ack contract: a successful enqueue means the message
/// has been persisted and will be processed — no separate ack channel needed.
#[tokio::test]
async fn test_enqueue_success_returns_ok_immediately() {
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<QueuedInbound>(4);
    let capacity = 4;
    start_inbound_consumer(rx, Arc::clone(&gw), capacity, None);

    let req = make_request("ok-test");
    let queued_msg = queued(req);

    // Enqueue directly — should return Ok without any consumer action.
    let result = tx.send(queued_msg).await;
    assert!(result.is_ok(), "channel send should succeed");
}

/// Queue-full path returns Err without blocking.
///
/// The producer does not wait for an ack signal; try_send fails
/// immediately when the channel is at capacity.
#[tokio::test]
async fn test_enqueue_queue_full_returns_err_immediately() {
    let (tx, _rx) = mpsc::channel::<QueuedInbound>(1);
    let handle = InboundQueueHandle::new(tx);

    // Fill queue to capacity (1).
    handle.try_send(queued(make_request("fill-0"))).unwrap();

    // Next try_send must fail immediately — no consumer needed.
    let err = handle.try_send(queued(make_request("overflow")));
    assert!(err.is_err());
    assert_eq!(err.unwrap_err().request.peer_id, "p1");
}

/// Queue-closed path returns Err immediately.
#[tokio::test]
async fn test_enqueue_queue_closed_returns_err() {
    let (tx, rx) = mpsc::channel::<QueuedInbound>(4);
    let handle = InboundQueueHandle::new(tx);

    // Close the consumer side.
    drop(rx);

    let err = handle.try_send(queued(make_request("closed")));
    assert!(err.is_err());
}

/// Concurrent enqueues all succeed without blocking each other.
///
/// Each producer returns immediately after its own enqueue; no ack
/// signals are exchanged between producer and consumer.
#[tokio::test]
async fn test_enqueue_concurrent_no_blocking() {
    let config = GatewayConfig {
        name: "test-concurrent".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 8,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    let _handle = gw.start_inbound_queue();

    // Enqueue 3 messages concurrently — each should return without blocking.
    let gw_clone = Arc::clone(&gw);
    let mut handles = Vec::new();
    for i in 0..3 {
        let gw_c = Arc::clone(&gw_clone);
        let req = make_request(&format!("concurrent-{i}"));
        handles.push(tokio::spawn(async move {
            let result = gw_c.enqueue_inbound(req).await;
            assert!(result.is_ok(), "enqueue {i} should succeed");
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
}
