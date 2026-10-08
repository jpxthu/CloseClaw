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

use crate::outbound_middleware::register::register_default;
use crate::outbound_raw_log::OutboundRawLogWriter;
use crate::RebuildStash;
use crate::{Gateway, GatewayConfig, SessionManager};

impl Gateway {
    /// Create a new Gateway with the given config, SessionManager, an
    /// injected processor chain and the simplified-path raw-log writer.
    ///
    /// Both collaborators are assembled by the composition root
    /// (daemon / root crate) — the Gateway never assembles concrete
    /// processors or raw-log writers itself.
    pub fn new(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        processor_chain: Arc<dyn ProcessorChain>,
        outbound_raw_log: Option<Arc<dyn OutboundRawLogWriter>>,
    ) -> Self {
        Self::build(
            config,
            session_manager,
            Some(processor_chain),
            outbound_raw_log,
        )
    }

    /// Create a Gateway **without** a processor chain and without a raw-log
    /// writer (bypass path).
    ///
    /// Test-only convenience: production call sites must inject both
    /// collaborators explicitly via [`Gateway::new`].
    pub fn new_for_tests(config: GatewayConfig, session_manager: Arc<SessionManager>) -> Self {
        Self::build(config, session_manager, None, None)
    }

    /// Create a new Gateway with the given config, SessionManager and
    /// ProcessorRegistry, without a raw-log writer.
    ///
    /// Test convenience for chain-driven tests; production call sites use
    /// [`Gateway::new`] so both collaborators are injected explicitly.
    pub fn with_processor_registry(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        registry: Arc<dyn ProcessorChain>,
    ) -> Self {
        Self::build(config, session_manager, Some(registry), None)
    }

    /// Shared constructor: `processor_chain = None` selects the chain-less
    /// bypass path, `outbound_raw_log = None` skips the simplified-path
    /// raw-log write.
    fn build(
        config: GatewayConfig,
        session_manager: Arc<SessionManager>,
        processor_chain: Option<Arc<dyn ProcessorChain>>,
        outbound_raw_log: Option<Arc<dyn OutboundRawLogWriter>>,
    ) -> Self {
        let gw = Self {
            config,
            plugins: RwLock::new(HashMap::new()),
            session_manager,
            processor_registry: std::sync::RwLock::new(processor_chain),
            outbound_raw_log,
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
        register_default(&gw, &gw.config);
        gw
    }
}
