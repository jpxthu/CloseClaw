//! Host-facing ports (dependency-inversion boundaries) owned by this crate.
//!
//! `im_adapter` is a domain-layer crate and must not depend on the gateway
//! crate (`STANDARDS.md` 依赖方向允许边表).
//! The composition root (daemon) wraps its gateway handle in the ports
//! defined here and injects them — together with the debug log — when
//! platform plugins are registered.
//!
//! The ports live here (not in `common`) because only this crate consumes
//! them; see issue #3347.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::IMPlugin;
use closeclaw_debug_log::DebugLog;

/// Field-level projection of the host's inbound request.
///
/// Carries exactly the fields the platform plugins fill in when handing an
/// event to the host queue — the host adapter maps it back onto its own
/// request type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundPayload {
    /// IM platform identifier (e.g. `"feishu"`).
    pub platform: String,
    /// Raw event payload bytes.
    pub raw_payload: Vec<u8>,
    /// Peer / chat ID (empty for long-connection events).
    pub peer_id: String,
    /// Trace ID generated at event arrival.
    pub trace_id: String,
    /// Root span ID for debug-log child span derivation.
    pub span_id: Option<String>,
}

/// Reason an inbound payload could not be enqueued.
///
/// Deliberately free of host-side types: the platform plugins only ever
/// see this local error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("inbound enqueue failed: {reason}")]
pub struct EnqueueError {
    /// Human-readable failure reason.
    pub reason: String,
}

impl EnqueueError {
    /// Build an error carrying the given reason.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Outbound port: hand an inbound payload to the host's queue.
#[async_trait]
pub trait InboundEnqueuer: Send + Sync {
    /// Enqueue `payload`; `Err` means the payload was rejected (queue full
    /// or closed) and the caller must log — never panic.
    async fn enqueue(&self, payload: InboundPayload) -> Result<(), EnqueueError>;
}

/// Outbound port: register an IM plugin with the host registry.
#[async_trait]
pub trait PluginRegistrar: Send + Sync {
    /// Register `plugin` under its [`platform`](IMPlugin::platform) key.
    async fn register_plugin(&self, plugin: Arc<dyn IMPlugin>);
}

/// Host-side ports injected by the composition root when platform plugins
/// are registered.
///
/// Bundles the two outbound ports plus the optional [`DebugLog`] so that
/// registration stays within the project's six-parameter limit; it replaces
/// the gateway handle the plugins used to receive directly.
#[derive(Clone)]
pub struct GatewayHost {
    /// Queue used for long-connection events.
    pub enqueuer: Arc<dyn InboundEnqueuer>,
    /// Plugin registry used at startup registration.
    pub registrar: Arc<dyn PluginRegistrar>,
    /// Debug log configured on the host, if any.
    pub debug_log: Option<DebugLog>,
}

#[cfg(test)]
pub(crate) mod test_doubles {
    //! Shared fakes for the host ports (registration + enqueue assertions).

    use super::*;
    use std::sync::Mutex;

    /// Records every payload handed to it; can be told to reject them.
    pub(crate) struct FakeEnqueuer {
        payloads: Mutex<Vec<InboundPayload>>,
        fail_reason: Option<String>,
        attempts: Mutex<usize>,
    }

    impl FakeEnqueuer {
        /// Accept every payload and record it.
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self {
                payloads: Mutex::new(Vec::new()),
                fail_reason: None,
                attempts: Mutex::new(0),
            })
        }

        /// Reject every payload with `reason`.
        pub(crate) fn failing(reason: &str) -> Arc<Self> {
            Arc::new(Self {
                payloads: Mutex::new(Vec::new()),
                fail_reason: Some(reason.to_string()),
                attempts: Mutex::new(0),
            })
        }

        /// Snapshot of the payloads recorded so far.
        pub(crate) fn payloads(&self) -> Vec<InboundPayload> {
            self.payloads
                .lock()
                .expect("payloads lock poisoned")
                .clone()
        }

        /// Number of `enqueue` calls observed so far (failed ones included).
        pub(crate) fn attempts(&self) -> usize {
            *self.attempts.lock().expect("attempts lock poisoned")
        }
    }

    #[async_trait]
    impl InboundEnqueuer for FakeEnqueuer {
        async fn enqueue(&self, payload: InboundPayload) -> Result<(), EnqueueError> {
            *self.attempts.lock().expect("attempts lock poisoned") += 1;
            if let Some(reason) = &self.fail_reason {
                return Err(EnqueueError::new(reason.clone()));
            }
            self.payloads
                .lock()
                .expect("payloads lock poisoned")
                .push(payload);
            Ok(())
        }
    }

    /// Records every plugin registered through it.
    ///
    /// The `park` variant parks forever inside [`PluginRegistrar::register_plugin`]
    /// so a registration future can never advance past registration (used by
    /// end-to-end registration tests to avoid spawning the real `lark-cli`).
    pub(crate) struct FakeRegistrar {
        registered: Mutex<Vec<Arc<dyn IMPlugin>>>,
        park: bool,
    }

    impl FakeRegistrar {
        /// Start with an empty registry; registrations complete normally.
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self {
                registered: Mutex::new(Vec::new()),
                park: false,
            })
        }

        /// Start with an empty registry; every registration parks forever.
        pub(crate) fn parking() -> Arc<Self> {
            Arc::new(Self {
                registered: Mutex::new(Vec::new()),
                park: true,
            })
        }

        /// Snapshot of the plugins registered so far.
        pub(crate) fn registered(&self) -> Vec<Arc<dyn IMPlugin>> {
            self.registered
                .lock()
                .expect("registered lock poisoned")
                .clone()
        }
    }

    #[async_trait]
    impl PluginRegistrar for FakeRegistrar {
        async fn register_plugin(&self, plugin: Arc<dyn IMPlugin>) {
            self.registered
                .lock()
                .expect("registered lock poisoned")
                .push(plugin);
            if self.park {
                std::future::pending::<()>().await;
            }
        }
    }

    /// Poll until `enqueuer` recorded at least `min_count` payloads
    /// (3-second cap) and return the recorded snapshot.
    pub(crate) async fn wait_payloads(
        enqueuer: &FakeEnqueuer,
        min_count: usize,
    ) -> Vec<InboundPayload> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let payloads = enqueuer.payloads();
            if payloads.len() >= min_count || tokio::time::Instant::now() >= deadline {
                return payloads;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Poll until `enqueuer` recorded at least `min_count` payloads
    /// (3-second cap) and return the recorded count.
    pub(crate) async fn wait_enqueued(enqueuer: &FakeEnqueuer, min_count: usize) -> usize {
        wait_payloads(enqueuer, min_count).await.len()
    }
}
