//! Unit tests for shutdown phase Gateway methods (Steps 1.1–1.4 alignment).
//!
//! Covers:
//! - `close_outbound()`: plugin shutdown_outbound + registry cleanup
//! - `sync_storage()`: delegates to PersistenceService::sync()
//! - `close_storage()`: delegates to PersistenceService::close()

use crate::{DmScope, GatewayConfig, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_common::shutdown::{DrainStatus, ShutdownSignal, ShutdownState};

use crate::shutdown_handle::ShutdownHandle;
use closeclaw_session::persistence::{PersistenceError, ReasoningLevel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ── Mock infrastructure ──────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

/// Mock plugin that tracks shutdown_outbound calls.
struct OutboundTrackerPlugin {
    platform: String,
    outbound_shutdown_called: std::sync::atomic::AtomicBool,
}

impl OutboundTrackerPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            outbound_shutdown_called: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl IMPlugin for OutboundTrackerPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        Ok(None)
    }

    async fn send(
        &self,
        _output: &closeclaw_common::im_plugin::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        Ok(())
    }

    async fn shutdown_outbound(&self) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        self.outbound_shutdown_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

/// Mock persistence service that tracks sync() and close() calls.
struct TrackingPersistService {
    sync_called: std::sync::atomic::AtomicBool,
    close_called: std::sync::atomic::AtomicBool,
}

impl TrackingPersistService {
    fn new() -> Self {
        Self {
            sync_called: std::sync::atomic::AtomicBool::new(false),
            close_called: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl closeclaw_session::persistence::PersistenceService for TrackingPersistService {
    async fn save_checkpoint(
        &self,
        _cp: &closeclaw_session::persistence::SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<closeclaw_session::persistence::SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn delete_checkpoint(&self, _id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }

    async fn sync(&self) -> Result<(), PersistenceError> {
        self.sync_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn close(&self) -> Result<(), PersistenceError> {
        self.close_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// close_outbound() tests
// ═════════════════════════════════════════════════════════════════════════════

/// close_outbound() calls shutdown_outbound() on all registered plugins.
#[tokio::test]
async fn test_close_outbound_calls_shutdown_outbound_on_plugins() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);

    let plugin_a = Arc::new(OutboundTrackerPlugin::new("alpha"));
    let plugin_b = Arc::new(OutboundTrackerPlugin::new("beta"));
    gw.register_plugin(plugin_a.clone()).await;
    gw.register_plugin(plugin_b.clone()).await;

    assert_eq!(gw.get_all_plugins().await.len(), 2);

    gw.close_outbound().await;

    assert!(
        plugin_a
            .outbound_shutdown_called
            .load(std::sync::atomic::Ordering::SeqCst),
        "plugin alpha shutdown_outbound should be called"
    );
    assert!(
        plugin_b
            .outbound_shutdown_called
            .load(std::sync::atomic::Ordering::SeqCst),
        "plugin beta shutdown_outbound should be called"
    );
}

/// close_outbound() clears the plugin registry after shutdown.
#[tokio::test]
async fn test_close_outbound_clears_plugin_registry() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);

    gw.register_plugin(Arc::new(OutboundTrackerPlugin::new("p1")))
        .await;
    gw.register_plugin(Arc::new(OutboundTrackerPlugin::new("p2")))
        .await;
    assert_eq!(gw.get_all_plugins().await.len(), 2);

    gw.close_outbound().await;

    assert_eq!(
        gw.get_all_plugins().await.len(),
        0,
        "plugin registry should be cleared after close_outbound"
    );
}

/// close_outbound() drops the processor chain (registry becomes None).
#[tokio::test]
async fn test_close_outbound_clears_processor_registry() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);

    let (i, o) = gw.processor_registry_len();
    assert_eq!(i + o, 0, "no processor registry initially");

    gw.close_outbound().await;

    let (i, o) = gw.processor_registry_len();
    assert_eq!(i + o, 0, "processor registry should be cleared");
}

// ═════════════════════════════════════════════════════════════════════════════
// sync_storage() tests
// ═════════════════════════════════════════════════════════════════════════════

/// sync_storage() delegates to PersistenceService::sync().
#[tokio::test]
async fn test_sync_storage_delegates_to_persistence_sync() {
    let tracking = Arc::new(TrackingPersistService::new());
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(tracking.clone() as Arc<dyn closeclaw_session::persistence::PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);

    gw.sync_storage().await.unwrap();

    assert!(
        tracking
            .sync_called
            .load(std::sync::atomic::Ordering::SeqCst),
        "PersistenceService::sync() should be called via sync_storage"
    );
}

/// sync_storage() returns Ok when no storage is configured.
#[tokio::test]
async fn test_sync_storage_noop_without_storage() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);

    gw.sync_storage().await.unwrap();
}

// ═════════════════════════════════════════════════════════════════════════════
// close_storage() tests
// ═════════════════════════════════════════════════════════════════════════════

/// close_storage() delegates to PersistenceService::close().
#[tokio::test]
async fn test_close_storage_delegates_to_persistence_close() {
    let tracking = Arc::new(TrackingPersistService::new());
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(tracking.clone() as Arc<dyn closeclaw_session::persistence::PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);

    gw.close_storage().await.unwrap();

    assert!(
        tracking
            .close_called
            .load(std::sync::atomic::Ordering::SeqCst),
        "PersistenceService::close() should be called via close_storage"
    );
}

/// close_storage() returns Ok when no storage is configured.
#[tokio::test]
async fn test_close_storage_noop_without_storage() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);

    gw.close_storage().await.unwrap();
}

// ═════════════════════════════════════════════════════════════════════════════
// Card action forceful shutdown escalation tests (Step 1.2)
// ═════════════════════════════════════════════════════════════════════════════

/// Mock `ShutdownSignal` that simulates CAS behavior for escalation.
///
/// The real `ShutdownCoordinator::escalate_to_forceful` uses an atomic
/// CAS to accept `ShuttingDown`/`Draining`/`Stopped` as source states
/// and reject `Running`/`ForcefulShuttingDown`. This mock reproduces
/// that contract for unit testing without a full coordinator.
struct MockStateSignal {
    state: ShutdownState,
    escalated: AtomicBool,
}

impl MockStateSignal {
    fn new(state: ShutdownState) -> Self {
        Self {
            state,
            escalated: AtomicBool::new(false),
        }
    }
}

impl ShutdownSignal for MockStateSignal {
    fn is_shutting_down(&self) -> bool {
        self.state.is_shutting_down_state()
    }

    fn increment_busy(&self) {}
    fn decrement_busy(&self) {}
    fn busy_count(&self) -> usize {
        0
    }

    fn escalate_to_forceful(&self) -> bool {
        // CAS: accept ShuttingDown/Draining/Stopped, reject Running/ForcefulShuttingDown
        if matches!(
            self.state,
            ShutdownState::ShuttingDown | ShutdownState::Draining | ShutdownState::Stopped
        ) {
            self.escalated.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    fn is_forceful(&self) -> bool {
        self.state == ShutdownState::ForcefulShuttingDown || self.escalated.load(Ordering::SeqCst)
    }

    fn drain_status(&self) -> DrainStatus {
        DrainStatus {
            state: self.state,
            busy_count: 0,
            is_draining: self.state == ShutdownState::Draining,
        }
    }
}

/// Card action escalation from `Stopped` state succeeds.
///
/// Phase 1 drain completes, state transitions to `Stopped`. The user
/// clicks "Forceful shutdown" on the progress card. The CAS should
/// accept `Stopped` as a valid source state.
#[test]
fn test_card_action_forceful_shutdown_from_stopped() {
    let signal = Arc::new(MockStateSignal::new(ShutdownState::Stopped));
    let handle = ShutdownHandle::new(signal.clone());

    assert!(handle.escalate_to_forceful());
    assert!(signal.escalated.load(Ordering::SeqCst));
    assert!(handle.is_forceful());
}

/// Card action escalation from `Running` state returns false (safe).
///
/// If the card action somehow fires before shutdown starts, the CAS
/// rejects `Running` and returns false. No state change occurs.
#[test]
fn test_card_action_forceful_shutdown_from_running() {
    let signal = Arc::new(MockStateSignal::new(ShutdownState::Running));
    let handle = ShutdownHandle::new(signal.clone());

    assert!(!handle.escalate_to_forceful());
    assert!(!signal.escalated.load(Ordering::SeqCst));
    assert!(!handle.is_forceful());
}

/// Card action escalation when already forceful returns false (no-op).
///
/// If the user double-clicks or a second SIGTERM already triggered
/// forceful mode, the CAS rejects `ForcefulShuttingDown` and returns
/// false. No redundant state change.
#[test]
fn test_card_action_forceful_shutdown_already_forceful() {
    let signal = Arc::new(MockStateSignal::new(ShutdownState::ForcefulShuttingDown));
    let handle = ShutdownHandle::new(signal.clone());

    assert!(!handle.escalate_to_forceful());
    assert!(!signal.escalated.load(Ordering::SeqCst));
    // is_forceful returns true because the state is ForcefulShuttingDown
    assert!(handle.is_forceful());
}
