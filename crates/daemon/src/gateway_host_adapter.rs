//! Composition-root shell: wraps `Arc<Gateway>` in `im_adapter`'s ports.
//!
//! `closeclaw-im-adapter` does not depend on `closeclaw-gateway`
//! (STANDARDS.md 依赖方向允许边表); the daemon wraps the gateway handle in
//! the [`InboundEnqueuer`] / [`PluginRegistrar`] ports and hands the bundle
//! to `register_platform_plugins` (issue #3347).

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::IMPlugin;
use closeclaw_gateway::inbound_queue::InboundRequest;
use closeclaw_gateway::Gateway;
use closeclaw_im_adapter::ports::{
    EnqueueError, GatewayHost, InboundEnqueuer, InboundPayload, PluginRegistrar,
};

/// [`InboundEnqueuer`] → `Gateway::enqueue_inbound`, mapping the
/// port-local payload onto the gateway's request type field by field.
pub(crate) struct GatewayInboundEnqueuer {
    gateway: Arc<Gateway>,
}

impl GatewayInboundEnqueuer {
    /// Wrap `gateway`.
    pub(crate) fn new(gateway: Arc<Gateway>) -> Self {
        Self { gateway }
    }
}

/// Map a port payload onto the gateway's request type, field by field.
fn to_inbound_request(payload: InboundPayload) -> InboundRequest {
    InboundRequest {
        platform: payload.platform,
        raw_payload: payload.raw_payload,
        peer_id: payload.peer_id,
        trace_id: payload.trace_id,
        span_id: payload.span_id,
    }
}

#[async_trait]
impl InboundEnqueuer for GatewayInboundEnqueuer {
    async fn enqueue(&self, payload: InboundPayload) -> Result<(), EnqueueError> {
        self.gateway
            .enqueue_inbound(to_inbound_request(payload))
            .await
            .map_err(|e| EnqueueError::new(e.to_string()))
    }
}

/// [`PluginRegistrar`] → `Gateway::register_plugin`.
pub(crate) struct GatewayPluginRegistrar {
    gateway: Arc<Gateway>,
}

impl GatewayPluginRegistrar {
    /// Wrap `gateway`.
    pub(crate) fn new(gateway: Arc<Gateway>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl PluginRegistrar for GatewayPluginRegistrar {
    async fn register_plugin(&self, plugin: Arc<dyn IMPlugin>) {
        self.gateway.register_plugin(plugin).await;
    }
}

/// Build the [`GatewayHost`] port bundle injected into platform
/// registration: both ports wrap `gateway`, the debug log is the one
/// currently configured on it.
pub(crate) fn gateway_host(gateway: &Arc<Gateway>) -> GatewayHost {
    GatewayHost {
        enqueuer: Arc::new(GatewayInboundEnqueuer::new(Arc::clone(gateway))),
        registrar: Arc::new(GatewayPluginRegistrar::new(Arc::clone(gateway))),
        debug_log: gateway.get_debug_log(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::{AdapterError, NormalizedMessage, RenderedOutput};
    use closeclaw_gateway::{GatewayConfig, SessionManager};
    use closeclaw_session::persistence::ReasoningLevel;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Gateway with a capacity-1 inbound queue and no WAL (keeps the test
    /// inside the sandbox — the default WAL dir lives under `~`).
    fn make_gateway() -> Arc<Gateway> {
        let config = GatewayConfig {
            name: "test".to_string(),
            inbound_queue_capacity: 1,
            inbound_wal_dir: None,
            ..Default::default()
        };
        let session_manager = Arc::new(SessionManager::new(
            &config,
            None,
            None,
            ReasoningLevel::default(),
        ));
        Arc::new(Gateway::new_for_tests(config, session_manager))
    }

    /// IMPlugin double that parks forever inside `parse_inbound` so the
    /// inbound consumer holds the first dequeued message and stops draining.
    struct ParkingPlugin {
        platform: String,
        parse_called: AtomicBool,
    }

    impl ParkingPlugin {
        fn new(platform: &str) -> Self {
            Self {
                platform: platform.to_string(),
                parse_called: AtomicBool::new(false),
            }
        }
    }

    #[async_trait]
    impl IMPlugin for ParkingPlugin {
        fn platform(&self) -> &str {
            &self.platform
        }

        async fn parse_inbound(
            &self,
            _payload: &[u8],
        ) -> Result<Option<NormalizedMessage>, AdapterError> {
            self.parse_called.store(true, Ordering::SeqCst);
            std::future::pending::<()>().await;
            Ok(None)
        }

        async fn send(
            &self,
            _output: &RenderedOutput,
            _peer_id: &str,
            _thread_id: Option<&str>,
            _reply_ref: Option<&str>,
        ) -> Result<(), AdapterError> {
            Ok(())
        }
    }

    fn payload(platform: &str, peer_id: &str) -> InboundPayload {
        InboundPayload {
            platform: platform.to_string(),
            raw_payload: b"{\"event\":{}}".to_vec(),
            peer_id: peer_id.to_string(),
            trace_id: "feishu_trace_1".to_string(),
            span_id: Some("span_1".to_string()),
        }
    }

    /// `InboundPayload` → `InboundRequest` copies every field verbatim.
    #[test]
    fn to_inbound_request_copies_all_five_fields() {
        let payload = payload("feishu", "oc_chat");
        let request = to_inbound_request(payload.clone());
        assert_eq!(request.platform, payload.platform);
        assert_eq!(request.raw_payload, payload.raw_payload);
        assert_eq!(request.peer_id, payload.peer_id);
        assert_eq!(request.trace_id, payload.trace_id);
        assert_eq!(request.span_id, payload.span_id);

        // `span_id: None` is preserved as-is too.
        let payload = InboundPayload {
            span_id: None,
            ..payload
        };
        assert_eq!(to_inbound_request(payload).span_id, None);
    }

    /// A queue-full rejection is mapped onto the port-local `EnqueueError`
    /// (no gateway types leak through the port).
    #[tokio::test]
    async fn enqueue_maps_queue_full_to_enqueue_error() {
        let gateway = make_gateway();
        let plugin = Arc::new(ParkingPlugin::new("feishu"));
        gateway
            .register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
            .await;
        let _handle = gateway.start_inbound_queue();
        let enqueuer = GatewayInboundEnqueuer::new(Arc::clone(&gateway));

        // First payload: the consumer dequeues it and parks in `parse_inbound`,
        // so it stops draining the queue.
        assert!(enqueuer.enqueue(payload("feishu", "p1")).await.is_ok());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while !plugin.parse_called.load(Ordering::SeqCst) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "consumer never started parsing the first payload"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Second payload fills the capacity-1 queue; third is rejected.
        assert!(enqueuer.enqueue(payload("feishu", "p2")).await.is_ok());
        let error = enqueuer
            .enqueue(payload("feishu", "p3"))
            .await
            .expect_err("queue is full — enqueue must be rejected");
        assert_eq!(error.reason, "inbound queue is full");
    }

    /// `register_plugin` forwards the plugin to the wrapped gateway.
    #[tokio::test]
    async fn register_plugin_forwards_to_gateway() {
        let gateway = make_gateway();
        let registrar = GatewayPluginRegistrar::new(Arc::clone(&gateway));
        registrar
            .register_plugin(Arc::new(ParkingPlugin::new("test_platform")) as Arc<dyn IMPlugin>)
            .await;

        let registered = gateway.get_plugin("test_platform").await;
        assert!(
            registered.is_some(),
            "plugin must reach the gateway registry"
        );
        assert_eq!(registered.unwrap().platform(), "test_platform");
        assert!(gateway.get_plugin("other_platform").await.is_none());
    }
}
