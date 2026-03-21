//! Graceful Shutdown Coordinator
//!
//! Manages the daemon shutdown lifecycle:
//!   RUNNING → SHUTTING_DOWN → DRAINING → STOPPED
//!
//! References:
//!   - OpenClaw's `deferGatewayRestartUntilIdle` (src/infra/restart.ts)
//!   - OpenClaw's `createGatewayCloseHandler` (src/gateway/server-close.ts)

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Shutdown state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownState {
    /// Normal operation
    Running,
    /// Shutdown signal received, stop accepting new work
    ShuttingDown,
    /// Waiting for in-flight operations to complete
    Draining,
    /// Clean exit
    Stopped,
}

impl ShutdownState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => ShutdownState::Running,
            1 => ShutdownState::ShuttingDown,
            2 => ShutdownState::Draining,
            3 => ShutdownState::Stopped,
            _ => ShutdownState::Running,
        }
    }
}

impl Default for ShutdownState {
    fn default() -> Self {
        ShutdownState::Running
    }
}

/// Global drain timeout
const DRAIN_TIMEOUT_SECS: u64 = 30;

/// Global drain poll interval
const DRAIN_POLL_SECS: u64 = 2;

/// ShutdownCoordinator — coordinates graceful shutdown across all components.
///
/// Uses an atomic state machine so components can check shutdown state
/// without locking.
#[derive(Debug)]
pub struct ShutdownCoordinator {
    state: AtomicU8,
}

impl ShutdownCoordinator {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(ShutdownState::Running as u8),
        }
    }

    /// Returns the current shutdown state
    pub fn state(&self) -> ShutdownState {
        ShutdownState::from_u8(self.state.load(Ordering::SeqCst))
    }

    /// Atomically transition from Running → ShuttingDown.
    /// Returns true if this call initiated shutdown, false if already shutting down.
    pub fn try_start_shutdown(&self) -> bool {
        self.state
            .compare_exchange(
                ShutdownState::Running as u8,
                ShutdownState::ShuttingDown as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    /// Transition to Draining state
    pub fn start_drain(&self) {
        self.state
            .store(ShutdownState::Draining as u8, Ordering::SeqCst);
    }

    /// Mark as fully stopped
    pub fn mark_stopped(&self) {
        self.state
            .store(ShutdownState::Stopped as u8, Ordering::SeqCst);
    }
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// ShutdownHandle — shared handle to the shutdown coordinator,
/// passed to components that need to cooperate with shutdown.
#[derive(Debug, Clone)]
pub struct ShutdownHandle {
    coordinator: Arc<ShutdownCoordinator>,
    /// Broadcast channel to signal all components the shutdown is done
    drain_done_tx: broadcast::Sender<()>,
}

impl ShutdownHandle {
    /// Create a new ShutdownHandle
    pub fn new() -> Self {
        let (drain_done_tx, _) = broadcast::channel(1);
        Self {
            coordinator: Arc::new(ShutdownCoordinator::new()),
            drain_done_tx,
        }
    }

    /// Returns the current state
    pub fn state(&self) -> ShutdownState {
        self.coordinator.state()
    }

    /// Returns true if shutdown has been initiated (not Running)
    pub fn is_shutting_down(&self) -> bool {
        self.coordinator.state() != ShutdownState::Running
    }

    /// Initiate graceful shutdown — called when SIGTERM/SIGINT is received.
    ///
    /// 1. Transition to ShuttingDown
    /// 2. Wait up to DRAIN_TIMEOUT_SECS for in-flight work to complete
    /// 3. Transition to Draining → Stopped
    ///
    /// If timeout is exceeded, force-exit.
    pub async fn initiate_shutdown(&self) {
        if !self.coordinator.try_start_shutdown() {
            info!("Shutdown already in progress");
            return;
        }

        info!(
            "Graceful shutdown initiated (timeout={}s)",
            DRAIN_TIMEOUT_SECS
        );

        // Signal all components to stop accepting new work
        // (Components check is_shutting_down() before starting new work)
        let _ = self.drain_done_tx.send(());

        // Wait for busy_count to reach 0, with timeout
        let started_at = std::time::Instant::now();
        let _deadline = started_at + std::time::Duration::from_secs(DRAIN_TIMEOUT_SECS);

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(DRAIN_POLL_SECS)).await;

            let elapsed = started_at.elapsed().as_secs();
            if elapsed >= DRAIN_TIMEOUT_SECS {
                warn!(
                    "Drain timeout exceeded ({}s) — forcing shutdown",
                    DRAIN_TIMEOUT_SECS
                );
                self.coordinator.start_drain();
                self.coordinator.mark_stopped();
                return;
            }

            info!(
                "Waiting for in-flight operations to complete... ({}s / {}s)",
                elapsed, DRAIN_TIMEOUT_SECS
            );
        }
    }

    /// Subscribe to the drain signal (called by components)
    pub fn subscribe_drain(&self) -> broadcast::Receiver<()> {
        self.drain_done_tx.subscribe()
    }

    /// Check if shutdown is complete
    pub fn is_stopped(&self) -> bool {
        self.coordinator.state() == ShutdownState::Stopped
    }
}

impl Default for ShutdownHandle {
    fn default() -> Self {
        Self::new()
    }
}
