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

#[async_trait]
impl InboundEnqueuer for GatewayInboundEnqueuer {
    async fn enqueue(&self, payload: InboundPayload) -> Result<(), EnqueueError> {
        let request = InboundRequest {
            platform: payload.platform,
            raw_payload: payload.raw_payload,
            peer_id: payload.peer_id,
            trace_id: payload.trace_id,
            span_id: payload.span_id,
        };
        self.gateway
            .enqueue_inbound(request)
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
