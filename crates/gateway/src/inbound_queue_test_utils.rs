//! Shared test utilities for inbound queue tests.

use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig, InboundRequest};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;
use tokio::sync::oneshot;

use super::inbound_queue::QueuedInbound;

pub fn make_raw_payload(text: &str) -> Vec<u8> {
    serde_json::json!({
        "header": {
            "event_id": "ev_test",
            "event_type": "im.message.receive_v1",
            "create_time": "1700000000000",
            "token": "t",
            "app_id": "a"
        },
        "event": {
            "sender": {
                "sender_id": {
                    "open_id": "u1"
                },
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
                "content": format!("{{\"text\":\"{}\"}}", text)
            }
        }
    })
    .to_string()
    .into_bytes()
}

pub fn make_request(content: &str) -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        raw_payload: make_raw_payload(content),
        peer_id: "p1".into(),
        trace_id: String::new(),
    }
}

/// Wrap an `InboundRequest` in a [`QueuedInbound`] with a dummy ack sender.
/// For tests that only care about the request payload, not the ack signal.
pub fn queued(request: InboundRequest) -> QueuedInbound {
    let (ack_tx, _) = oneshot::channel();
    QueuedInbound { request, ack_tx }
}

pub fn make_gateway() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 4,
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
