//! Gateway construction.
//!
//! The Gateway never assembles concrete processors itself: the composition
//! root (daemon / root crate) injects the chain as a common `ProcessorChain`
//! trait object.

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_common::processor::ProcessorChain;
use closeclaw_config::MediaConfigData;
use tokio::sync::RwLock;

use crate::default_middlewares::register_default_middlewares;
use crate::RebuildStash;
use crate::{Gateway, GatewayConfig, SessionManager};

impl Gateway {
    /// Create a new Gateway with the given config, SessionManager and an
    /// injected processor chain.
    ///
    /// The chain is built by the composition root (daemon / root crate) — the
    /// Gateway never assembles concrete processors itself.
    pub fn new(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        processor_chain: Arc<dyn ProcessorChain>,
    ) -> Self {
        Self::with_processor_registry(config, session_manager, processor_chain)
    }

    /// Create a Gateway **without** a processor chain (bypass path).
    ///
    /// Test-only convenience: production call sites must inject the chain
    /// explicitly via [`Gateway::new`].
    pub fn new_for_tests(config: GatewayConfig, session_manager: Arc<SessionManager>) -> Self {
        Self::with_processor_chain_opt(config, session_manager, None)
    }

    /// Create a new Gateway with the given config, SessionManager and ProcessorRegistry.
    pub fn with_processor_registry(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        registry: Arc<dyn ProcessorChain>,
    ) -> Self {
        Self::with_processor_chain_opt(config, session_manager, Some(registry))
    }

    /// Shared constructor: `registry = None` selects the chain-less bypass path.
    fn with_processor_chain_opt(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        registry: Option<Arc<dyn ProcessorChain>>,
    ) -> Self {
        let gw = Self {
            config,
            plugins: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: std::sync::RwLock::new(registry),
            checkpoint_manager: std::sync::RwLock::new(None),
            session_handler: std::sync::OnceLock::new(),
            approval_flow: RwLock::new(None),
            plan_confirm_handler: RwLock::new(None),
            slash_dispatcher: RwLock::new(None),
            permission_engine: RwLock::new(None),
            inbound_tx: std::sync::Mutex::new(None),
            self_ref: std::sync::Mutex::new(None),
            shutdown_handle: std::sync::Mutex::new(None),
            outbound_middlewares: std::sync::RwLock::new(Vec::new()),
            config_dir: RwLock::new(None),
            metrics_emitter: std::sync::RwLock::new(None),
            debug_log: std::sync::RwLock::new(None),
            inbound_wal: std::sync::Mutex::new(None),
            rebuild_stash: Arc::new(RebuildStash::new()),
            media_store: std::sync::Mutex::new(None),
            media_config: std::sync::RwLock::new(MediaConfigData::default()),
        };
        register_default_middlewares(&gw, &gw.config);
        gw
    }
}
