//! Graceful Shutdown Coordinator
//!
//! Manages the daemon shutdown lifecycle:
//!   RUNNING → SHUTTING_DOWN → DRAINING → STOPPED
//!
//! References:
//!   - OpenClaw's `deferGatewayRestartUntilIdle` (src/infra/restart.ts)
//!   - OpenClaw's `createGatewayCloseHandler` (src/gateway/server-close.ts)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{info, warn};

// Re-export types defined in closeclaw-common.
pub use closeclaw_common::{DrainStatus, ShutdownMode, ShutdownState};

/// Returns the drain poll interval.
#[cfg(not(test))]
pub(crate) const fn drain_poll_interval() -> std::time::Duration {
    std::time::Duration::from_secs(2)
}

/// Returns the drain poll interval (test mode: 100ms).
#[cfg(test)]
pub(crate) const fn drain_poll_interval() -> std::time::Duration {
    std::time::Duration::from_millis(100)
}

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

    /// Atomically transition from Running → ForcefulShuttingDown.
    /// Returns true if this call initiated forceful shutdown, false if
    /// not in Running state.
    pub fn try_start_forceful_shutdown(&self) -> bool {
        self.state
            .compare_exchange(
                ShutdownState::Running as u8,
                ShutdownState::ForcefulShuttingDown as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    /// Atomically escalate from any active shutdown state to ForcefulShuttingDown.
    ///
    /// Accepts `ShuttingDown`, `Draining`, and `Stopped` as source states.
    /// Returns `true` if the escalation succeeded, `false` if already in
    /// `ForcefulShuttingDown` or not yet shutting down (`Running`).
    pub fn escalate_to_forceful(&self) -> bool {
        for _ in 0..2 {
            let current = self.state.load(Ordering::SeqCst);
            match ShutdownState::from_u8(current) {
                ShutdownState::ForcefulShuttingDown | ShutdownState::Running => {
                    return false;
                }
                _ => {
                    if self
                        .state
                        .compare_exchange(
                            current,
                            ShutdownState::ForcefulShuttingDown as u8,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_ok()
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Returns the current shutdown mode.
    pub fn mode(&self) -> ShutdownMode {
        ShutdownState::from_u8(self.state.load(Ordering::SeqCst)).mode()
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
    /// Counter for in-flight operations — components increment before starting
    /// async work and decrement when complete. Drains exits early when 0.
    busy_count: Arc<AtomicUsize>,
    /// Maximum time to wait for in-flight operations before proceeding
    /// to Phase 2. Default: 30 seconds.
    drain_timeout: Duration,
    /// Tracked operation descriptions keyed by unique id.
    pending_descriptions: Arc<Mutex<HashMap<u64, String>>>,
    /// Monotonic id generator for tracked operations.
    next_tracked_id: Arc<AtomicU64>,
}

impl ShutdownHandle {
    /// Create a new ShutdownHandle with default drain timeout (30s).
    pub fn new() -> Self {
        let (drain_done_tx, _) = broadcast::channel(1);
        Self {
            coordinator: Arc::new(ShutdownCoordinator::new()),
            drain_done_tx,
            busy_count: Arc::new(AtomicUsize::new(0)),
            drain_timeout: Duration::from_secs(30),
            pending_descriptions: Arc::new(Mutex::new(HashMap::new())),
            next_tracked_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Builder method: set a custom drain timeout.
    pub fn with_drain_timeout(mut self, timeout: Duration) -> Self {
        self.drain_timeout = timeout;
        self
    }

    /// Returns the current state
    pub fn state(&self) -> ShutdownState {
        self.coordinator.state()
    }

    /// Returns true if shutdown has been initiated (not Running)
    pub fn is_shutting_down(&self) -> bool {
        self.coordinator.state().is_shutting_down_state()
    }

    /// Returns true if the current shutdown is forceful.
    pub fn is_forceful(&self) -> bool {
        self.coordinator.state() == ShutdownState::ForcefulShuttingDown
    }

    /// Returns the current shutdown mode.
    pub fn mode(&self) -> ShutdownMode {
        self.coordinator.mode()
    }

    /// Atomically transition from Running → ShuttingDown.
    /// Returns true if this call initiated shutdown, false if already shutting down.
    pub fn try_start_shutdown(&self) -> bool {
        self.coordinator.try_start_shutdown()
    }

    /// Atomically transition from Running → ForcefulShuttingDown.
    /// Returns true if this call initiated forceful shutdown, false if
    /// not in Running state.
    pub fn try_start_forceful_shutdown(&self) -> bool {
        self.coordinator.try_start_forceful_shutdown()
    }

    /// Escalate a graceful shutdown to forceful.
    /// Accepts ShuttingDown, Draining, and Stopped as source states.
    /// Returns true if escalation succeeded, false if already forceful or Running.
    pub fn escalate_to_forceful(&self) -> bool {
        self.coordinator.escalate_to_forceful()
    }
}

impl ShutdownHandle {
    /// Initiate graceful shutdown — called when SIGTERM/SIGINT is received.
    ///
    /// 1. Transition to ShuttingDown
    /// 2. Wait for in-flight work to complete (no timeout)
    /// 3. Transition to Draining → Stopped
    ///
    /// If already shutting down, escalates to forceful.
    /// Only a forceful upgrade or busy_count reaching 0 can end the wait.
    ///
    /// Returns the remaining `busy_count` after drain completes.
    pub async fn initiate_shutdown(&self) -> usize {
        // Try to transition from Running → ShuttingDown
        if self.coordinator.try_start_shutdown() {
            // We initiated shutdown — normal graceful path
            info!(
                "Graceful shutdown initiated — waiting for in-flight operations \
                    (forceful via repeated signal)"
            );
            let _ = self.drain_done_tx.send(());
            return self.wait_for_drain().await;
        }

        // Already shutting down. If the gate was set by Phase 0 (graceful),
        // just proceed with drain without escalating.
        if !self.is_forceful() {
            info!("Shutdown gate already set — proceeding with drain");
            let _ = self.drain_done_tx.send(());
            return self.wait_for_drain().await;
        }

        // Already forceful — just drain
        info!("Forceful mode — drain");
        self.wait_for_drain().await
    }

    /// Wait for busy_count to reach 0 or timeout, then finalize shutdown.
    /// In forceful mode, finalize immediately without waiting.
    ///
    /// Timeout does not trigger forceful escalation — it merely ends the
    /// drain wait so the caller can proceed to Phase 2 normally.
    ///
    /// Returns the remaining `busy_count` (0 when all drained or forceful).
    async fn wait_for_drain(&self) -> usize {
        let start = tokio::time::Instant::now();

        loop {
            // If upgraded to forceful mid-drain, finalize immediately
            if self.is_forceful() {
                info!("Forceful mode — skipping drain wait");
                self.coordinator.start_drain();
                self.coordinator.mark_stopped();
                return 0;
            }

            let count = self.busy_count.load(Ordering::SeqCst);
            if count == 0 {
                info!("All in-flight operations complete, shutting down immediately");
                self.coordinator.start_drain();
                self.coordinator.mark_stopped();
                return 0;
            }

            if start.elapsed() >= self.drain_timeout {
                info!(
                    "Drain timeout ({:?}) — {} operations still in-flight",
                    self.drain_timeout, count
                );
                self.coordinator.start_drain();
                self.coordinator.mark_stopped();
                return count;
            }

            info!("Waiting for in-flight operations... (busy_count={})", count);

            tokio::time::sleep(drain_poll_interval()).await;
        }
    }
}

impl ShutdownHandle {
    /// Subscribe to the drain signal (called by components)
    pub fn subscribe_drain(&self) -> broadcast::Receiver<()> {
        self.drain_done_tx.subscribe()
    }

    /// Check if shutdown is complete
    pub fn is_stopped(&self) -> bool {
        self.coordinator.state() == ShutdownState::Stopped
    }
}

#[cfg(test)]
impl ShutdownHandle {
    pub fn start_shutdown_for_test(&self) {
        self.coordinator.try_start_shutdown();
    }

    pub fn set_draining_for_test(&self) {
        self.coordinator.start_drain();
    }

    pub fn set_stopped_for_test(&self) {
        self.coordinator.mark_stopped();
    }
}

impl ShutdownHandle {
    /// Increment the busy count (call before starting async work)
    pub fn increment_busy(&self) {
        self.busy_count.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrement the busy count (call after async work completes)
    pub fn decrement_busy(&self) {
        self.busy_count.fetch_sub(1, Ordering::SeqCst);
    }

    /// Get current busy count (for debugging/monitoring)
    pub fn busy_count(&self) -> usize {
        self.busy_count.load(Ordering::SeqCst)
    }

    /// Returns a structured snapshot of the current drain status.
    pub fn drain_status(&self) -> DrainStatus {
        self.drain_status_snapshot()
    }

    /// Internal helper that captures a snapshot of the current drain status.
    fn drain_status_snapshot(&self) -> DrainStatus {
        let pending = match self.pending_descriptions.lock() {
            Ok(map) => map.values().cloned().collect(),
            Err(e) => {
                warn!(error = %e, "failed to lock pending_descriptions for drain_status snapshot");
                Vec::new()
            }
        };
        DrainStatus {
            state: self.coordinator.state(),
            busy_count: self.busy_count(),
            is_draining: self.coordinator.state() == ShutdownState::Draining,
            pending_items: pending,
        }
    }

    /// Increment busy count with a tracked description.
    ///
    /// Returns a unique id that must be passed to [`decrement_busy_tracked`]
    /// to unregister.
    pub fn increment_busy_tracked(&self, desc: &str) -> u64 {
        let id = self.next_tracked_id.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut map) = self.pending_descriptions.lock() {
            map.insert(id, desc.to_owned());
        }
        self.increment_busy();
        id
    }

    /// Decrement busy count and remove the tracked description.
    pub fn decrement_busy_tracked(&self, id: u64) {
        if let Ok(mut map) = self.pending_descriptions.lock() {
            map.remove(&id);
        }
        self.decrement_busy();
    }
}

impl Default for ShutdownHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl closeclaw_common::ShutdownSignal for ShutdownHandle {
    fn is_shutting_down(&self) -> bool {
        self.coordinator.state().is_shutting_down_state()
    }

    fn increment_busy(&self) {
        self.busy_count.fetch_add(1, Ordering::SeqCst);
    }

    fn decrement_busy(&self) {
        self.busy_count.fetch_sub(1, Ordering::SeqCst);
    }

    fn busy_count(&self) -> usize {
        self.busy_count.load(Ordering::SeqCst)
    }

    fn escalate_to_forceful(&self) -> bool {
        self.coordinator.escalate_to_forceful()
    }

    fn is_forceful(&self) -> bool {
        self.coordinator.state() == ShutdownState::ForcefulShuttingDown
    }

    fn drain_status(&self) -> DrainStatus {
        self.drain_status_snapshot()
    }

    fn increment_busy_tracked(&self, desc: &str) -> u64 {
        self.increment_busy_tracked(desc)
    }

    fn decrement_busy_tracked(&self, id: u64) {
        self.decrement_busy_tracked(id)
    }
}
