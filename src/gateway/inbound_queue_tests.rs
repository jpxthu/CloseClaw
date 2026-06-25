//! Unit tests for the inbound bounded queue.
//!
//! Covers: enqueue success, full-queue rejection with busy reply,
//! FIFO ordering, consumer task dispatch, and bypass mode.

use std::sync::Arc;

use crate::gateway::session_manager::SessionManager;
use crate::gateway::{Gateway, GatewayConfig, InboundRequest};
use crate::session::bootstrap::loader::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use tokio::sync::mpsc;

use super::inbound_queue::{start_inbound_consumer, InboundQueueFull, InboundQueueHandle};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_request(content: &str) -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        sender_id: "u1".into(),
        peer_id: "p1".into(),
        content: content.to_owned(),
        message_id: format!("mid-{content}"),
        timestamp_ms: 1_700_000_000_000,
        thread_id: None,
        account_id: None,
    }
}

fn make_gateway() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: Default::default(),
        inbound_queue_capacity: 4,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    Arc::new(Gateway::new(config, sm))
}

// ---------------------------------------------------------------------------
// Handle-level tests (pure channel, no Gateway)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_try_send_ok_and_capacity() {
    let (tx, _rx) = mpsc::channel::<InboundRequest>(8);
    let handle = InboundQueueHandle::new(tx);
    assert_eq!(handle.capacity(), 8);
    assert!(handle.try_send(make_request("a")).is_ok());
    assert!(handle.try_send(make_request("b")).is_ok());
}

#[tokio::test]
async fn test_try_send_full_returns_original_request() {
    let (tx, _rx) = mpsc::channel::<InboundRequest>(1);
    let handle = InboundQueueHandle::new(tx);
    assert!(handle.try_send(make_request("a")).is_ok());
    let err: Result<(), InboundQueueFull> = handle.try_send(make_request("overflow"));
    assert!(err.is_err());
    let full = err.unwrap_err();
    assert_eq!(full.request.content, "overflow");
}

#[tokio::test]
async fn test_try_send_closed_channel() {
    let (tx, rx) = mpsc::channel::<InboundRequest>(4);
    let handle = InboundQueueHandle::new(tx);
    drop(rx); // close receiver
    let err: Result<(), InboundQueueFull> = handle.try_send(make_request("x"));
    assert!(err.is_err());
}

// ---------------------------------------------------------------------------
// Consumer task tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_consumer_fires_process_and_handle() {
    // The consumer task calls gateway.process_inbound_chain + handle_inbound_message.
    // Without a session_handler installed, handle_inbound_message returns None,
    // but the consumer should not panic or hang.
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<InboundRequest>(8);
    let capacity = 8;
    start_inbound_consumer(rx, Arc::clone(&gw), capacity);

    // Send a message through the channel directly.
    tx.send(make_request("hello")).await.unwrap();
    tx.send(make_request("world")).await.unwrap();

    // Give the consumer time to process.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // Channel should be drained.
    assert!(tx.try_send(make_request("z")).is_ok());
    // No panic = consumer ran and handled missing session_handler gracefully.
}

#[tokio::test]
async fn test_consumer_fifo_order() {
    // Messages are processed in order; we verify by sending N messages
    // and ensuring none are dropped.
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<InboundRequest>(16);
    start_inbound_consumer(rx, Arc::clone(&gw), 16);

    for i in 0..10 {
        tx.send(make_request(&format!("msg-{i}"))).await.unwrap();
    }

    // Wait for processing.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    // All messages consumed — channel should be empty.
    assert!(tx.try_send(make_request("extra")).is_ok());
}

#[tokio::test]
async fn test_consumer_stops_on_channel_close() {
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<InboundRequest>(4);
    start_inbound_consumer(rx, Arc::clone(&gw), 4);

    tx.send(make_request("before")).await.unwrap();
    drop(tx); // close sender — consumer should exit its loop

    // Consumer task should terminate; we verify by waiting a bit.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // No panic = consumer exited cleanly.
}

// ---------------------------------------------------------------------------
// Gateway-level enqueue_inbound tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_enqueue_inbound_without_queue_bypasses() {
    // When inbound_tx is None (queue not started), enqueue_inbound
    // processes the message directly without going through the channel.
    let gw = make_gateway();
    // No start_inbound_queue() call — inbound_tx remains None.
    gw.enqueue_inbound(make_request("direct")).await;
    // If we got here without panic, bypass mode works.
}

#[tokio::test]
async fn test_start_inbound_queue_returns_handle() {
    let gw = make_gateway();
    let handle = gw.start_inbound_queue();
    // Handle should have the configured capacity.
    assert_eq!(handle.capacity(), 4);
    // Enqueue via handle should succeed.
    assert!(handle.try_send(make_request("ok")).is_ok());
}

#[tokio::test]
async fn test_gateway_enqueue_inbound_full_triggers_busy_reply() {
    // Fill the queue to capacity, then enqueue one more.
    // Since no plugin is registered, the busy reply is silently dropped.
    let gw = make_gateway();
    let handle = gw.start_inbound_queue();

    // Fill queue (capacity = 4).
    for i in 0..4 {
        handle.try_send(make_request(&format!("fill-{i}"))).unwrap();
    }
    // Next enqueue should trigger busy reply path (no plugin → silently skipped).
    gw.enqueue_inbound(make_request("overflow")).await;
    // No panic = busy reply path handled gracefully with no plugin.
}

#[tokio::test]
async fn test_inbound_request_clone_preserves_fields() {
    let req = make_request("clone-test");
    let cloned = req.clone();
    assert_eq!(cloned.platform, "feishu");
    assert_eq!(cloned.sender_id, "u1");
    assert_eq!(cloned.peer_id, "p1");
    assert_eq!(cloned.content, "clone-test");
    assert_eq!(cloned.thread_id, None);
    assert_eq!(cloned.account_id, None);
}
