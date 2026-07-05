//! Inbound bounded queue for buffering IM webhook messages.
//!
//! The queue sits between IM platform webhooks and the Processor Chain,
//! providing a bounded buffer that protects the Gateway from burst traffic.
//! When the queue is full, new messages are rejected with a busy reply.

use std::sync::Arc;
use tokio::sync::mpsc;

use super::Gateway;
use closeclaw_common::MessageType;

/// An inbound message awaiting processing.
///
/// Stores the raw webhook payload so the consumer task can parse it
/// through the IM plugin _after_ entering the queue, matching the
/// design doc architecture where the queue sits before plugin parsing.
///
/// `peer_id` is stored separately for the busy-reply path (when the
/// queue is full, we need a target to reply to without parsing).
#[derive(Debug, Clone)]
pub struct InboundRequest {
    /// IM platform identifier (e.g. "feishu", "discord").
    pub platform: String,
    /// Raw webhook payload bytes.
    pub raw_payload: Vec<u8>,
    /// Peer / chat ID — used for busy-reply when the queue is full.
    pub peer_id: String,
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
/// each message through the IM plugin parser, processor chain, and
/// inbound handler.
///
/// The task runs until the receiver is closed (Gateway shutdown).
///
/// Flow per message:
/// 1. Get the registered IM plugin for `platform`
/// 2. Call `plugin.parse_inbound(raw_payload)` → try NormalizedMessage
/// 3. If None, call `plugin.parse_card_action(raw_payload)` → try CardActionEvent
/// 4. Route: NormalizedMessage → inbound chain → handle; CardActionEvent → handle_card_action
///
/// When the plugin is not registered or both parsers return `None`, the
/// message is silently dropped.
pub(crate) fn start_inbound_consumer(
    mut rx: mpsc::Receiver<InboundRequest>,
    gateway: Arc<Gateway>,
    capacity: usize,
) {
    tokio::spawn(async move {
        tracing::info!(capacity, "inbound queue consumer started");
        while let Some(req) = rx.recv().await {
            // ── 1. Resolve plugin ─────────────────────────────────────
            let Some(plugin) = gateway.get_plugin(&req.platform).await else {
                tracing::warn!(
                    platform = %req.platform,
                    "inbound consumer: no plugin registered — dropping"
                );
                continue;
            };

            // ── 2. Try parsing as NormalizedMessage ───────────────────
            match plugin.parse_inbound(&req.raw_payload).await {
                Ok(Some(normalized)) => {
                    // ── 2a. Process through inbound chain ───────────────
                    let sender_id = normalized.sender_id.clone();
                    let input = super::InboundChainInput {
                        platform: normalized.platform.clone(),
                        sender_id: normalized.sender_id.clone(),
                        peer_id: normalized.peer_id.clone(),
                        content: normalized.content.clone(),
                        message_id: String::new(),
                        timestamp_ms: normalized.timestamp,
                        account_id: Some(normalized.account_id.clone()),
                        thread_id: normalized.thread_id.clone(),
                        message_type: normalized.message_type.clone(),
                        media_refs: normalized.media_refs.clone(),
                    };
                    let processed = gateway.process_inbound_chain(&input).await;

                    // ── 2b. Handle inbound message ──────────────────────
                    gateway
                        .handle_inbound_message(processed, Some(&sender_id), &normalized.platform)
                        .await;
                    continue;
                }
                Ok(None) => { /* not a message — try card action below */ }
                Err(e) => {
                    tracing::warn!(
                        platform = %req.platform,
                        peer_id = %req.peer_id,
                        error = %e,
                        "inbound consumer: parse_inbound failed — dropping"
                    );
                    continue;
                }
            }

            // ── 3. Try parsing as CardActionEvent ──────────────────────
            match plugin.parse_card_action(&req.raw_payload).await {
                Ok(Some(card_action)) => {
                    // Card actions bypass the inbound Processor Chain and are
                    // injected directly as tool-result payloads.
                    gateway.handle_card_action(card_action).await;
                }
                Ok(None) => {
                    tracing::debug!(
                        platform = %req.platform,
                        peer_id = %req.peer_id,
                        "inbound consumer: no match (message or card action) — dropping"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        platform = %req.platform,
                        peer_id = %req.peer_id,
                        error = %e,
                        "inbound consumer: parse_card_action failed — dropping"
                    );
                }
            }
        }
        tracing::info!("inbound queue consumer stopped");
    });
}

/// Reply text sent when the inbound queue is at capacity.
const BUSY_REPLY_TEXT: &str =
    "\u{274C} \u{670D}\u{52A1}\u{7E41}\u{5FD9}\u{FF0C}\u{8BF7}\u{7A0D}\u{540E}\u{91CD}\u{8BD5}";

/// Try to enqueue an inbound request into the gateway's bounded queue.
///
/// On success the request will be processed by the consumer task.
/// When the queue is at capacity, a busy reply is sent to the user
/// via the registered IM plugin and the request is dropped.
///
/// When the queue has not been started (fallback mode), the raw payload
/// is parsed inline and processed immediately.
pub(crate) async fn enqueue_inbound(gateway: &Gateway, request: InboundRequest) {
    let tx = match gateway
        .inbound_tx
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned())
    {
        Some(tx) => tx,
        None => {
            process_inbound_direct(gateway, &request).await;
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

/// Fallback: process an inbound request directly when the queue has not started.
///
/// Parses the raw payload through the IM plugin, runs the processor chain,
/// and handles the inbound message inline.
async fn process_inbound_direct(gateway: &Gateway, request: &InboundRequest) {
    tracing::warn!("inbound queue not started — processing directly");
    let Some(plugin) = gateway.get_plugin(&request.platform).await else {
        tracing::warn!(
            platform = %request.platform,
            "inline fallback: no plugin registered — dropping"
        );
        return;
    };
    // Try NormalizedMessage first.
    match plugin.parse_inbound(&request.raw_payload).await {
        Ok(Some(normalized)) => {
            // Defensive: drop empty text messages that slipped through parse_inbound.
            // Per design doc: "text type empty content messages are discarded at parse
            // stage, no NormalizedMessage produced".
            if normalized.message_type == MessageType::Text && normalized.content.trim().is_empty()
            {
                tracing::debug!(
                    peer_id = %request.peer_id,
                    "dropping empty text message"
                );
                return;
            }
            let sender_id = normalized.sender_id.clone();
            let input = super::InboundChainInput {
                platform: normalized.platform.clone(),
                sender_id: normalized.sender_id.clone(),
                peer_id: normalized.peer_id.clone(),
                content: normalized.content.clone(),
                message_id: String::new(),
                timestamp_ms: normalized.timestamp,
                account_id: Some(normalized.account_id.clone()),
                thread_id: normalized.thread_id.clone(),
                message_type: normalized.message_type.clone(),
                media_refs: normalized.media_refs.clone(),
            };
            let processed = gateway.process_inbound_chain(&input).await;
            gateway
                .handle_inbound_message(processed, Some(&sender_id), &normalized.platform)
                .await;
            return;
        }
        Ok(None) => { /* not a message — try card action below */ }
        Err(e) => {
            tracing::warn!(
                platform = %request.platform,
                error = %e,
                "inline fallback: parse_inbound failed — dropping"
            );
            return;
        }
    }

    // Try CardActionEvent second.
    match plugin.parse_card_action(&request.raw_payload).await {
        Ok(Some(card_action)) => {
            gateway.handle_card_action(card_action).await;
        }
        Ok(None) => {
            tracing::debug!(
                platform = %request.platform,
                "inline fallback: no match (message or card action) — dropping"
            );
        }
        Err(e) => {
            tracing::warn!(
                platform = %request.platform,
                error = %e,
                "inline fallback: parse_card_action failed — dropping"
            );
        }
    }
}

/// Send a "service busy" reply via the outbound Processor Chain.
///
/// The reply text goes through the outbound chain (DslParser → RawLog)
/// and is then rendered by the IM plugin, consistent with slash-command
/// and LLM reply paths.
async fn send_busy_reply(gateway: &Gateway, request: &InboundRequest) {
    if let Err(e) = gateway
        .send_outbound_to_chat(&request.peer_id, &request.platform, BUSY_REPLY_TEXT)
        .await
    {
        tracing::warn!(error = %e, "failed to send busy reply");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_request_fields() {
        let req = InboundRequest {
            platform: "feishu".into(),
            raw_payload: b"{\"event\":{}}".to_vec(),
            peer_id: "p1".into(),
        };
        assert_eq!(req.platform, "feishu");
        assert_eq!(req.raw_payload, b"{\"event\":{}}");
        assert_eq!(req.peer_id, "p1");
    }

    #[test]
    fn inbound_queue_handle_try_send_ok() {
        let (tx, _rx) = mpsc::channel::<InboundRequest>(2);
        let handle = InboundQueueHandle::new(tx);
        let req = InboundRequest {
            platform: "feishu".into(),
            raw_payload: b"hello".to_vec(),
            peer_id: "p1".into(),
        };
        assert!(handle.try_send(req).is_ok());
    }

    #[test]
    fn inbound_queue_handle_try_send_full() {
        let (tx, _rx) = mpsc::channel::<InboundRequest>(1);
        let handle = InboundQueueHandle::new(tx);
        let req1 = InboundRequest {
            platform: "feishu".into(),
            raw_payload: b"a".to_vec(),
            peer_id: "p1".into(),
        };
        let req2 = InboundRequest {
            platform: "feishu".into(),
            raw_payload: b"b".to_vec(),
            peer_id: "p2".into(),
        };
        assert!(handle.try_send(req1).is_ok());
        let err = handle.try_send(req2);
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().request.peer_id, "p2");
    }

    #[test]
    fn inbound_queue_handle_capacity() {
        let (tx, _rx) = mpsc::channel::<InboundRequest>(32);
        let handle = InboundQueueHandle::new(tx);
        assert_eq!(handle.capacity(), 32);
    }
}
