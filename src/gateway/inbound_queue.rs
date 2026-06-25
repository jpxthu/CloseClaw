//! Inbound bounded queue for buffering IM webhook messages.
//!
//! The queue sits between IM platform webhooks and the Processor Chain,
//! providing a bounded buffer that protects the Gateway from burst traffic.
//! When the queue is full, new messages are rejected with a busy reply.

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::renderer::RenderedOutput;

use super::Gateway;

/// An inbound message awaiting processing.
///
/// Carries all fields needed by `process_inbound_chain` and
/// `handle_inbound_message` so the consumer task can replay the
/// full inbound path without the original webhook context.
#[derive(Debug, Clone)]
pub struct InboundRequest {
    /// IM platform identifier (e.g. "feishu", "discord").
    pub platform: String,
    /// Sender's user ID on the platform.
    pub sender_id: String,
    /// Peer / chat ID the message was sent to.
    pub peer_id: String,
    /// Message content (text).
    pub content: String,
    /// Platform-specific message ID for deduplication.
    pub message_id: String,
    /// Message timestamp in milliseconds since epoch.
    pub timestamp_ms: i64,
    /// Optional thread / topic ID.
    pub thread_id: Option<String>,
    /// Optional account identifier (for multi-account setups).
    pub account_id: Option<String>,
}

/// Handle to the inbound queue producer side.
///
/// Wraps the [`mpsc::Sender`] so callers only need to call
/// [`try_send`](InboundQueueHandle::try_send) without knowing the
/// channel internals.
pub struct InboundQueueHandle {
    tx: mpsc::Sender<InboundRequest>,
}

impl InboundQueueHandle {
    /// Create a new handle from a channel sender.
    #[allow(dead_code)]
    pub(crate) fn new(tx: mpsc::Sender<InboundRequest>) -> Self {
        Self { tx }
    }

    /// Try to enqueue an inbound request without blocking.
    ///
    /// Returns `Ok(())` on success, or `Err(full)` when the queue is at
    /// capacity. The caller should reply with a busy message on `Err`.
    #[allow(clippy::result_large_err)]
    pub fn try_send(&self, request: InboundRequest) -> Result<(), InboundQueueFull> {
        match self.tx.try_send(request) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(req))
            | Err(tokio::sync::mpsc::error::TrySendError::Closed(req)) => {
                Err(InboundQueueFull { request: req })
            }
        }
    }

    /// Returns the channel capacity.
    pub fn capacity(&self) -> usize {
        self.tx.capacity()
    }
}

/// Error returned when the inbound queue is full.
///
/// Contains the original request so the caller can decide what to do
/// (e.g. log it, drop it, or reply with a busy message).
#[derive(Debug)]
pub struct InboundQueueFull {
    /// The request that could not be enqueued.
    pub request: InboundRequest,
}

/// Spawn a consumer task that drains the inbound queue and processes
/// each message through the processor chain and inbound handler.
///
/// The task runs until the receiver is closed (Gateway shutdown).
pub(crate) fn start_inbound_consumer(
    mut rx: mpsc::Receiver<InboundRequest>,
    gateway: Arc<Gateway>,
    capacity: usize,
) {
    tokio::spawn(async move {
        tracing::info!(capacity, "inbound queue consumer started");
        while let Some(req) = rx.recv().await {
            let processed = gateway
                .process_inbound_chain(
                    &req.platform,
                    &req.sender_id,
                    &req.peer_id,
                    &req.content,
                    &req.message_id,
                    req.timestamp_ms,
                )
                .await;
            gateway
                .handle_inbound_message(processed, Some(&req.sender_id), &req.platform)
                .await;
        }
        tracing::info!("inbound queue consumer stopped");
    });
}

/// Try to enqueue an inbound request into the gateway's bounded queue.
///
/// On success the request will be processed by the consumer task.
/// When the queue is at capacity, a busy reply is sent to the user
/// via the registered IM plugin and the request is dropped.
pub(crate) async fn enqueue_inbound(gateway: &Gateway, request: InboundRequest) {
    let tx = match gateway
        .inbound_tx
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned())
    {
        Some(tx) => tx,
        None => {
            tracing::warn!("inbound queue not started — processing directly");
            let processed = gateway
                .process_inbound_chain(
                    &request.platform,
                    &request.sender_id,
                    &request.peer_id,
                    &request.content,
                    &request.message_id,
                    request.timestamp_ms,
                )
                .await;
            gateway
                .handle_inbound_message(processed, Some(&request.sender_id), &request.platform)
                .await;
            return;
        }
    };

    match tx.try_send(request.clone()) {
        Ok(()) => {}
        Err(e) => {
            let req = match e {
                tokio::sync::mpsc::error::TrySendError::Full(r)
                | tokio::sync::mpsc::error::TrySendError::Closed(r) => r,
            };
            tracing::warn!(peer_id = %req.peer_id, "inbound queue full — sending busy reply");
            send_busy_reply(gateway, &req).await;
        }
    }
}

/// Send a "service busy" reply via the IM plugin for the request's platform.
async fn send_busy_reply(gateway: &Gateway, request: &InboundRequest) {
    if let Some(plugin) = gateway.get_plugin(&request.platform).await {
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({
                "content": {
                    "text": "\u{274C} \u{670D}\u{52A1}\u{7E41}\u{5FD9}\u{FF0C}\u{8BF7}\u{7A0D}\u{540E}\u{91CD}\u{8BD5}"
                }
            }),
        };
        if let Err(e) = plugin.send(&output, &request.peer_id, None).await {
            tracing::warn!(error = %e, "failed to send busy reply");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_request_fields() {
        let req = InboundRequest {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: "hello".into(),
            message_id: "m1".into(),
            timestamp_ms: 1_700_000_000_000,
            thread_id: Some("t1".into()),
            account_id: Some("a1".into()),
        };
        assert_eq!(req.platform, "feishu");
        assert_eq!(req.sender_id, "u1");
        assert_eq!(req.peer_id, "p1");
        assert_eq!(req.content, "hello");
        assert_eq!(req.message_id, "m1");
        assert_eq!(req.timestamp_ms, 1_700_000_000_000);
        assert_eq!(req.thread_id.as_deref(), Some("t1"));
        assert_eq!(req.account_id.as_deref(), Some("a1"));
    }

    #[test]
    fn inbound_queue_handle_try_send_ok() {
        let (tx, _rx) = mpsc::channel::<InboundRequest>(2);
        let handle = InboundQueueHandle::new(tx);
        let req = InboundRequest {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: "hello".into(),
            message_id: "m1".into(),
            timestamp_ms: 0,
            thread_id: None,
            account_id: None,
        };
        assert!(handle.try_send(req).is_ok());
    }

    #[test]
    fn inbound_queue_handle_try_send_full() {
        let (tx, _rx) = mpsc::channel::<InboundRequest>(1);
        let handle = InboundQueueHandle::new(tx);
        let req1 = InboundRequest {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: "a".into(),
            message_id: "m1".into(),
            timestamp_ms: 0,
            thread_id: None,
            account_id: None,
        };
        let req2 = InboundRequest {
            platform: "feishu".into(),
            sender_id: "u2".into(),
            peer_id: "p2".into(),
            content: "b".into(),
            message_id: "m2".into(),
            timestamp_ms: 0,
            thread_id: None,
            account_id: None,
        };
        assert!(handle.try_send(req1).is_ok());
        let err = handle.try_send(req2);
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().request.content, "b");
    }

    #[test]
    fn inbound_queue_handle_capacity() {
        let (tx, _rx) = mpsc::channel::<InboundRequest>(32);
        let handle = InboundQueueHandle::new(tx);
        assert_eq!(handle.capacity(), 32);
    }
}
