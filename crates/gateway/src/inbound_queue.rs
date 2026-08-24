//! Inbound bounded queue for buffering IM webhook messages.
//!
//! The queue sits between IM platform webhooks and the Processor Chain,
//! providing a bounded buffer that protects the Gateway from burst traffic.
//! When the queue is full, new messages are rejected with a busy reply.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

use super::inbound_wal::{InboundWal, InboundWalEntry};
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
    /// Trace ID for debug-log correlation, generated at webhook arrival.
    pub trace_id: String,
}

/// An inbound request paired with a oneshot ack sender.
///
/// The consumer sends `()` through the oneshot after dequeuing the
/// request, allowing the producer (webhook handler) to ack the HTTP
/// response only after the message leaves the channel buffer.
pub(crate) struct QueuedInbound {
    /// The inbound request to enqueue.
    pub(crate) request: InboundRequest,
    /// Oneshot sender — consumer signals after dequeue.
    pub(crate) ack_tx: oneshot::Sender<()>,
}

/// Handle to the inbound queue producer side.
///
/// Wraps the [`mpsc::Sender`] so callers only need to call
/// [`try_send`](InboundQueueHandle::try_send) without knowing the
/// channel internals.
pub struct InboundQueueHandle {
    tx: mpsc::Sender<QueuedInbound>,
}

impl InboundQueueHandle {
    /// Create a new handle from a channel sender.
    #[allow(dead_code)]
    pub(crate) fn new(tx: mpsc::Sender<QueuedInbound>) -> Self {
        Self { tx }
    }

    /// Try to enqueue an inbound request without blocking.
    ///
    /// Returns `Ok(())` on success, or `Err(full)` when the queue is at
    /// capacity. The caller should reply with a busy message on `Err`.
    #[allow(clippy::result_large_err, dead_code)]
    pub(crate) fn try_send(&self, queued: QueuedInbound) -> Result<(), InboundQueueFull> {
        match self.tx.try_send(queued) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(q))
            | Err(tokio::sync::mpsc::error::TrySendError::Closed(q)) => {
                Err(InboundQueueFull { request: q.request })
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
#[derive(Debug, thiserror::Error)]
#[error("inbound queue is full")]
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
/// Process a parsed NormalizedMessage through the inbound chain and handle it.
///
/// Drops empty text messages defensively (per design doc: "text type empty
/// content messages are discarded at parse stage, no NormalizedMessage produced").
async fn handle_normalized_message(
    gateway: &Gateway,
    req: &InboundRequest,
    normalized: closeclaw_common::NormalizedMessage,
    plugin: &Arc<dyn closeclaw_common::IMPlugin>,
) {
    if normalized.message_type == MessageType::Text && normalized.content.trim().is_empty() {
        tracing::debug!(peer_id = %req.peer_id, "dropping empty text message");
        return;
    }
    // Retrieve platform-specific metadata from the adapter (e.g. chat_name).
    let adapter_meta = plugin.last_parsed_metadata();
    let chat_name = adapter_meta.get("chat_name").cloned().unwrap_or_default();
    let mut msg = normalized;
    msg.chat_name = chat_name;
    msg.trace_id = req.trace_id.clone();
    let processed = gateway.process_inbound_chain(&msg).await;
    gateway
        .handle_inbound_message(processed, Some(&msg.sender_id), &msg.platform)
        .await;
}

/// Process a single inbound request through plugin parsing and the inbound chain.
///
/// This is the core logic extracted from the consumer loop. It resolves the
/// plugin, parses the raw payload, and routes NormalizedMessage / CardActionEvent
/// appropriately.
async fn process_single_request(
    gateway: &Gateway,
    req: &InboundRequest,
    plugin: &Arc<dyn closeclaw_common::IMPlugin>,
) {
    let start = Instant::now();
    match plugin.parse_inbound(&req.raw_payload).await {
        Ok(Some(normalized)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            emit_inbound_parsed_log(
                gateway,
                &req.trace_id,
                &normalized.platform,
                &normalized.message_type,
                duration_ms,
            );
            handle_normalized_message(gateway, req, normalized, plugin).await;
        }
        Ok(None) => match plugin.parse_card_action(&req.raw_payload).await {
            Ok(Some(card_action)) => {
                gateway.handle_card_action(card_action).await;
            }
            Ok(None) => {
                tracing::debug!(
                    platform = %req.platform,
                    peer_id = %req.peer_id,
                    "no match (message or card action) — dropping"
                );
            }
            Err(e) => {
                tracing::warn!(
                    platform = %req.platform,
                    peer_id = %req.peer_id,
                    error = %e,
                    "parse_card_action failed — dropping"
                );
            }
        },
        Err(e) => {
            tracing::warn!(
                platform = %req.platform,
                peer_id = %req.peer_id,
                error = %e,
                "parse_inbound failed — dropping"
            );
        }
    }
}

/// Spawn a consumer task that drains the inbound queue and processes
/// each message through the IM plugin parser, processor chain, and
/// inbound handler.
///
/// The task runs until the receiver is closed (Gateway shutdown).
pub(crate) fn start_inbound_consumer(
    mut rx: mpsc::Receiver<QueuedInbound>,
    gateway: Arc<Gateway>,
    capacity: usize,
    wal: Option<Arc<InboundWal>>,
) {
    tokio::spawn(async move {
        tracing::info!(capacity, "inbound queue consumer started");
        while let Some(queued) = rx.recv().await {
            let req = queued.request;
            // Signal ack to producer — message has left the channel buffer.
            let _ = queued.ack_tx.send(());
            {
                let guard = gateway.debug_log.read().unwrap_or_else(|e| e.into_inner());
                super::debug_log_emitter::emit_debug_event(
                    guard.as_ref(),
                    &req.trace_id,
                    None,
                    closeclaw_debug_log::LogLevel::Info,
                    "gateway",
                    "queue.dequeued",
                    serde_json::json!({
                        "platform": req.platform,
                        "peer_id": req.peer_id,
                    }),
                );
            }
            let Some(plugin) = gateway.get_plugin(&req.platform).await else {
                tracing::warn!(
                    platform = %req.platform,
                    "inbound consumer: no plugin registered — dropping"
                );
                continue;
            };
            process_single_request(gateway.as_ref(), &req, &plugin).await;
            // WAL: mark entry done and remove after full processing.
            if let Some(ref wal) = wal {
                if let Err(e) = wal.mark_done_and_delete(&req.trace_id) {
                    tracing::warn!(
                        trace_id = %req.trace_id,
                        error = %e,
                        "WAL: failed to mark done after processing"
                    );
                }
            }
        }
        tracing::info!("inbound queue consumer stopped");
    });
}

/// Reply text sent when the inbound queue is at capacity.
const BUSY_REPLY_TEXT: &str =
    "\u{670D}\u{52A1}\u{7E41}\u{5FD9}\u{FF0C}\u{8BF7}\u{7A0D}\u{540E}\u{91CD}\u{8BD5}";

/// Try to enqueue an inbound request into the gateway's bounded queue.
///
/// On success the request will be processed by the consumer task.
/// When the queue is at capacity, a busy reply is sent to the user
/// via the registered IM plugin and the request is dropped.
///
/// When the queue has not been started (fallback mode), the raw payload
/// is parsed inline and processed immediately.
/// Ensure `request.trace_id` is non-empty for debug-log correlation.
///
/// If the caller did not provide a trace_id, a new one is generated from
/// the platform name, current timestamp, and a random UUID.
fn ensure_trace_id(request: &mut InboundRequest) {
    if request.trace_id.is_empty() {
        request.trace_id = format!(
            "{}-{}-{}",
            request.platform,
            chrono::Utc::now().timestamp_millis(),
            uuid::Uuid::new_v4(),
        );
    }
}

/// Emit a `feishu.inbound.parsed` debug event for successful inbound parsing.
fn emit_inbound_parsed_log(
    gateway: &Gateway,
    trace_id: &str,
    platform: &str,
    message_type: &closeclaw_common::MessageType,
    duration_ms: u64,
) {
    let guard = gateway.debug_log.read().unwrap_or_else(|e| e.into_inner());
    let msg_type_str = match message_type {
        closeclaw_common::MessageType::Text => "text",
        closeclaw_common::MessageType::Image => "image",
        closeclaw_common::MessageType::File => "file",
        closeclaw_common::MessageType::Audio => "audio",
    };
    super::debug_log_emitter::emit_debug_event(
        guard.as_ref(),
        trace_id,
        None,
        closeclaw_debug_log::LogLevel::Info,
        platform,
        "feishu.inbound.parsed",
        serde_json::json!({
            "platform": platform,
            "message_type": msg_type_str,
            "parse_duration_ms": duration_ms,
        }),
    );
}

/// Emit a debug event for queue-full rejections.
fn emit_queue_rejected_log(gateway: &Gateway, req: &InboundRequest) {
    let guard = gateway.debug_log.read().unwrap_or_else(|e| e.into_inner());
    super::debug_log_emitter::emit_debug_event(
        guard.as_ref(),
        &req.trace_id,
        None,
        closeclaw_debug_log::LogLevel::Warn,
        "gateway",
        "queue.rejected",
        serde_json::json!({
            "platform": req.platform,
            "peer_id": req.peer_id,
            "reason": "queue_full",
        }),
    );
}

/// Persist the inbound request to WAL if configured.
///
/// Failures are logged and do not block the enqueue path (best-effort
/// durability, consistent with the webhook-ack-before-processing design).
fn append_wal_if_configured(gateway: &Gateway, request: &InboundRequest) {
    let Ok(wal_guard) = gateway.inbound_wal.lock() else {
        return;
    };
    let Some(ref wal) = *wal_guard else {
        return;
    };
    let entry = InboundWalEntry::new(
        request.trace_id.clone(),
        request.platform.clone(),
        &request.raw_payload,
        request.peer_id.clone(),
    );
    if let Err(e) = wal.append(&entry) {
        tracing::warn!(
            trace_id = %request.trace_id,
            error = %e,
            "WAL: append failed — continuing with in-memory queue"
        );
    }
}

pub(crate) async fn enqueue_inbound(
    gateway: &Gateway,
    mut request: InboundRequest,
) -> Result<(), InboundQueueFull> {
    ensure_trace_id(&mut request);
    let tx = match gateway
        .inbound_tx
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned())
    {
        Some(tx) => tx,
        None => {
            process_inbound_direct(gateway, &request).await;
            return Ok(());
        }
    };

    append_wal_if_configured(gateway, &request);

    // Create oneshot channel for dequeue ack.
    let (ack_tx, _ack_rx) = oneshot::channel::<()>();
    let queued = QueuedInbound { request, ack_tx };

    match tx.try_send(queued) {
        Ok(()) => Ok(()),
        Err(e) => {
            let req = match e {
                tokio::sync::mpsc::error::TrySendError::Full(q)
                | tokio::sync::mpsc::error::TrySendError::Closed(q) => q.request,
            };
            emit_queue_rejected_log(gateway, &req);
            tracing::warn!(peer_id = %req.peer_id, "inbound queue full — sending busy reply");
            send_busy_reply(gateway, &req).await;
            Err(InboundQueueFull { request: req })
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
    let start = Instant::now();
    match plugin.parse_inbound(&request.raw_payload).await {
        Ok(Some(normalized)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            emit_inbound_parsed_log(
                gateway,
                &request.trace_id,
                &normalized.platform,
                &normalized.message_type,
                duration_ms,
            );
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
            // Retrieve platform-specific metadata from the adapter (e.g. chat_name).
            let adapter_meta = plugin.last_parsed_metadata();
            let chat_name = adapter_meta.get("chat_name").cloned().unwrap_or_default();
            let mut msg = normalized;
            msg.chat_name = chat_name;
            msg.trace_id = request.trace_id.clone();
            let processed = gateway.process_inbound_chain(&msg).await;
            gateway
                .handle_inbound_message(processed, Some(&msg.sender_id), &msg.platform)
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

/// Send a "service busy" reply via the simplified outbound path.
///
/// The reply text skips VerbosityFilter/DslParser/middleware and goes
/// directly through OutboundRawLog → render → send, per design doc:
/// "non-text error replies are sent via the simplified outbound path".
///
/// Per design doc: the reply must complete within 2 seconds to avoid
/// blocking the Gateway. If the send times out, we log and move on.
async fn send_busy_reply(gateway: &Gateway, request: &InboundRequest) {
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        gateway.send_outbound_simplified(&request.peer_id, &request.platform, BUSY_REPLY_TEXT),
    )
    .await;

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::warn!(
                peer_id = %request.peer_id,
                platform = %request.platform,
                error = %e,
                "failed to send busy reply"
            );
        }
        Err(_elapsed) => {
            tracing::warn!(
                peer_id = %request.peer_id,
                platform = %request.platform,
                "busy reply timed out after 2s — dropping"
            );
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
            raw_payload: b"{\"event\":{}}".to_vec(),
            peer_id: "p1".into(),
            trace_id: "feishu-123-tr".into(),
        };
        assert_eq!(req.platform, "feishu");
        assert_eq!(req.raw_payload, b"{\"event\":{}}");
        assert_eq!(req.peer_id, "p1");
        assert_eq!(req.trace_id, "feishu-123-tr");
    }

    #[test]
    fn inbound_queue_handle_try_send_ok() {
        let (tx, _rx) = mpsc::channel::<QueuedInbound>(2);
        let handle = InboundQueueHandle::new(tx);
        let (ack_tx, _ack_rx) = oneshot::channel();
        let queued = QueuedInbound {
            request: InboundRequest {
                platform: "feishu".into(),
                raw_payload: b"hello".to_vec(),
                peer_id: "p1".into(),
                trace_id: "tr-1".into(),
            },
            ack_tx,
        };
        assert!(handle.try_send(queued).is_ok());
    }

    #[test]
    fn inbound_queue_handle_try_send_full() {
        let (tx, _rx) = mpsc::channel::<QueuedInbound>(1);
        let handle = InboundQueueHandle::new(tx);
        let (ack_tx1, _ack_rx1) = oneshot::channel();
        let queued1 = QueuedInbound {
            request: InboundRequest {
                platform: "feishu".into(),
                raw_payload: b"a".to_vec(),
                peer_id: "p1".into(),
                trace_id: "tr-1".into(),
            },
            ack_tx: ack_tx1,
        };
        let (ack_tx2, _ack_rx2) = oneshot::channel();
        let queued2 = QueuedInbound {
            request: InboundRequest {
                platform: "feishu".into(),
                raw_payload: b"b".to_vec(),
                peer_id: "p2".into(),
                trace_id: "tr-2".into(),
            },
            ack_tx: ack_tx2,
        };
        assert!(handle.try_send(queued1).is_ok());
        let err = handle.try_send(queued2);
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().request.peer_id, "p2");
    }

    #[test]
    fn inbound_queue_handle_capacity() {
        let (tx, _rx) = mpsc::channel::<QueuedInbound>(32);
        let handle = InboundQueueHandle::new(tx);
        assert_eq!(handle.capacity(), 32);
    }

    /// Verify that enqueue does not block waiting for consumer dequeue.
    ///
    /// Fills the channel to capacity, then enqueues one more message.
    /// The first message is not consumed, so if enqueue blocked on ack_rx
    /// it would time out. Instead, try_send should return Err immediately
    /// because the channel is full.
    #[test]
    fn enqueue_does_not_block_without_consumer() {
        let (tx, rx) = mpsc::channel::<QueuedInbound>(1);
        let handle = InboundQueueHandle::new(tx);

        // Fill the single slot.
        let (ack_tx1, _ack_rx1) = oneshot::channel();
        handle
            .try_send(QueuedInbound {
                request: InboundRequest {
                    platform: "feishu".into(),
                    raw_payload: b"fill".to_vec(),
                    peer_id: "p1".into(),
                    trace_id: "tr-fill".into(),
                },
                ack_tx: ack_tx1,
            })
            .unwrap();

        // Channel is full. try_send must fail immediately — no consumer,
        // no ack_rx.await blocking.
        let (ack_tx2, _ack_rx2) = oneshot::channel();
        let err = handle.try_send(QueuedInbound {
            request: InboundRequest {
                platform: "feishu".into(),
                raw_payload: b"overflow".to_vec(),
                peer_id: "p2".into(),
                trace_id: "tr-overflow".into(),
            },
            ack_tx: ack_tx2,
        });
        assert!(err.is_err());

        // Drop consumer side without consuming — proves enqueue never
        // depended on consumer for its return path.
        drop(rx);
    }
}
