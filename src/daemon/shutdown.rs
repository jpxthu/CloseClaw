//! Graceful Shutdown Coordinator
//!
//! Manages the daemon shutdown lifecycle:
//!   RUNNING → SHUTTING_DOWN → DRAINING → STOPPED
//!
//! References:
//!   - OpenClaw's `deferGatewayRestartUntilIdle` (src/infra/restart.ts)
//!   - OpenClaw's `createGatewayCloseHandler` (src/gateway/server-close.ts)

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::info;

// ShutdownMode is now defined in closeclaw_common and re-exported.
pub use closeclaw_common::ShutdownMode;

/// Shutdown state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShutdownState {
    /// Normal operation
    #[default]
    Running,
    /// Shutdown signal received, stop accepting new work
    ShuttingDown,
    /// Waiting for in-flight operations to complete
    Draining,
    /// Clean exit
    Stopped,
    /// Forceful shutdown — skip drain, terminate immediately
    ForcefulShuttingDown,
}

impl ShutdownState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => ShutdownState::Running,
            1 => ShutdownState::ShuttingDown,
            2 => ShutdownState::Draining,
            3 => ShutdownState::Stopped,
            4 => ShutdownState::ForcefulShuttingDown,
            _ => ShutdownState::Running,
        }
    }

    /// Returns true if the state represents an active shutdown
    /// (either graceful or forceful).
    fn is_shutting_down_state(self) -> bool {
        matches!(
            self,
            ShutdownState::ShuttingDown
                | ShutdownState::Draining
                | ShutdownState::ForcefulShuttingDown
        )
    }

    /// Returns the shutdown mode for an active shutdown state.
    fn mode(self) -> ShutdownMode {
        match self {
            ShutdownState::ForcefulShuttingDown => ShutdownMode::Forceful,
            _ => ShutdownMode::Graceful,
        }
    }
}

/// Returns the drain poll interval.
#[cfg(not(test))]
const fn drain_poll_interval() -> std::time::Duration {
    std::time::Duration::from_secs(2)
}

/// Returns the drain poll interval (test mode: 100ms).
#[cfg(test)]
const fn drain_poll_interval() -> std::time::Duration {
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

    /// Atomically escalate from ShuttingDown → ForcefulShuttingDown.
    /// Returns true if the escalation succeeded, false if already in a
    /// non-ShuttingDown state.
    pub fn escalate_to_forceful(&self) -> bool {
        self.state
            .compare_exchange(
                ShutdownState::ShuttingDown as u8,
                ShutdownState::ForcefulShuttingDown as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
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

    /// Escalate a graceful shutdown to forceful.
    /// Returns true if escalation succeeded, false if not in ShuttingDown state.
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
    pub async fn initiate_shutdown(&self) {
        if !self.coordinator.try_start_shutdown() {
            // Already shutting down — escalate to forceful on repeated signal
            if self.escalate_to_forceful() {
                self.wait_for_drain().await;
            }
            return;
        }

        info!(
            "Graceful shutdown initiated — waiting for in-flight operations \
                (forceful via repeated signal)"
        );

        let _ = self.drain_done_tx.send(());
        self.wait_for_drain().await;
    }

    /// Wait for busy_count to reach 0 or timeout, then finalize shutdown.
    /// In forceful mode, finalize immediately without waiting.
    ///
    /// Timeout does not trigger forceful escalation — it merely ends the
    /// drain wait so the caller can proceed to Phase 2 normally.
    async fn wait_for_drain(&self) {
        let start = tokio::time::Instant::now();

        loop {
            // If upgraded to forceful mid-drain, finalize immediately
            if self.is_forceful() {
                info!("Forceful mode — skipping drain wait");
                self.coordinator.start_drain();
                self.coordinator.mark_stopped();
                return;
            }

            let count = self.busy_count.load(Ordering::SeqCst);
            if count == 0 {
                info!("All in-flight operations complete, shutting down immediately");
                self.coordinator.start_drain();
                self.coordinator.mark_stopped();
                return;
            }

            if start.elapsed() >= self.drain_timeout {
                info!(
                    "Drain timeout ({:?}) — {} operations still in-flight",
                    self.drain_timeout, count
                );
                self.coordinator.start_drain();
                self.coordinator.mark_stopped();
                return;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shutdown_state_from_u8() {
        assert_eq!(ShutdownState::from_u8(0), ShutdownState::Running);
        assert_eq!(ShutdownState::from_u8(1), ShutdownState::ShuttingDown);
        assert_eq!(ShutdownState::from_u8(2), ShutdownState::Draining);
        assert_eq!(ShutdownState::from_u8(3), ShutdownState::Stopped);
        assert_eq!(
            ShutdownState::from_u8(4),
            ShutdownState::ForcefulShuttingDown
        );
        // Invalid values default to Running
        assert_eq!(ShutdownState::from_u8(99), ShutdownState::Running);
    }

    #[test]
    fn test_coordinator_initial_state() {
        let coordinator = ShutdownCoordinator::new();
        assert_eq!(coordinator.state(), ShutdownState::Running);
    }

    #[test]
    fn test_coordinator_try_start_shutdown() {
        let coordinator = ShutdownCoordinator::new();

        // First call succeeds
        assert!(coordinator.try_start_shutdown());
        assert_eq!(coordinator.state(), ShutdownState::ShuttingDown);

        // Second call fails (already shutting down)
        assert!(!coordinator.try_start_shutdown());
        assert_eq!(coordinator.state(), ShutdownState::ShuttingDown);
    }

    #[test]
    fn test_coordinator_state_transitions() {
        let coordinator = ShutdownCoordinator::new();

        coordinator.try_start_shutdown();
        assert_eq!(coordinator.state(), ShutdownState::ShuttingDown);

        coordinator.start_drain();
        assert_eq!(coordinator.state(), ShutdownState::Draining);

        coordinator.mark_stopped();
        assert_eq!(coordinator.state(), ShutdownState::Stopped);
    }

    #[test]
    fn test_coordinator_escalate_to_forceful_success() {
        let coordinator = ShutdownCoordinator::new();
        coordinator.try_start_shutdown();
        assert!(coordinator.escalate_to_forceful());
        assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
    }

    #[test]
    fn test_coordinator_escalate_to_forceful_fails_when_running() {
        let coordinator = ShutdownCoordinator::new();
        assert!(!coordinator.escalate_to_forceful());
        assert_eq!(coordinator.state(), ShutdownState::Running);
    }

    #[test]
    fn test_coordinator_escalate_to_forceful_fails_when_stopped() {
        let coordinator = ShutdownCoordinator::new();
        coordinator.try_start_shutdown();
        coordinator.start_drain();
        coordinator.mark_stopped();
        assert!(!coordinator.escalate_to_forceful());
        assert_eq!(coordinator.state(), ShutdownState::Stopped);
    }

    #[test]
    fn test_coordinator_escalate_to_forceful_fails_when_already_forceful() {
        let coordinator = ShutdownCoordinator::new();
        coordinator.try_start_shutdown();
        assert!(coordinator.escalate_to_forceful());
        // Second escalate should fail (already forceful, not ShuttingDown)
        assert!(!coordinator.escalate_to_forceful());
        assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
    }

    #[test]
    fn test_coordinator_mode() {
        let coordinator = ShutdownCoordinator::new();
        assert_eq!(coordinator.mode(), ShutdownMode::Graceful);

        coordinator.try_start_shutdown();
        assert_eq!(coordinator.mode(), ShutdownMode::Graceful);

        coordinator.escalate_to_forceful();
        assert_eq!(coordinator.mode(), ShutdownMode::Forceful);
    }

    #[test]
    fn test_shutdown_handle_initial_state() {
        let handle = ShutdownHandle::new();
        assert_eq!(handle.state(), ShutdownState::Running);
        assert!(!handle.is_shutting_down());
        assert!(!handle.is_stopped());
        assert!(!handle.is_forceful());
        assert_eq!(handle.mode(), ShutdownMode::Graceful);
    }

    #[test]
    fn test_shutdown_handle_escalate_success() {
        let handle = ShutdownHandle::new();
        handle.coordinator.try_start_shutdown();
        assert!(handle.escalate_to_forceful());
        assert!(handle.is_forceful());
        assert_eq!(handle.mode(), ShutdownMode::Forceful);
    }

    #[test]
    fn test_shutdown_handle_escalate_fails_when_running() {
        let handle = ShutdownHandle::new();
        assert!(!handle.escalate_to_forceful());
        assert!(!handle.is_forceful());
    }

    #[test]
    fn test_shutdown_handle_is_shutting_down_in_forceful() {
        let handle = ShutdownHandle::new();
        handle.coordinator.try_start_shutdown();
        handle.escalate_to_forceful();
        assert!(handle.is_shutting_down());
    }

    #[test]
    fn test_shutdown_handle_subscribe_drain() {
        let handle = ShutdownHandle::new();
        let mut rx = handle.subscribe_drain();
        // No message yet
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_initiate_shutdown_first_caller_wins() {
        let handle = ShutdownHandle::new();
        // Register a busy operation so drain doesn't complete immediately
        handle.busy_count.fetch_add(1, Ordering::SeqCst);

        // First initiate succeeds
        let handle2 = handle.clone();
        tokio::spawn(async move {
            handle2.initiate_shutdown().await;
        });

        // Give it a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(handle.is_shutting_down());

        // Release the busy count so drain can complete
        handle.decrement_busy();
    }

    #[test]
    fn test_shutdown_state_debug() {
        assert_eq!(format!("{:?}", ShutdownState::Running), "Running");
        assert_eq!(format!("{:?}", ShutdownState::ShuttingDown), "ShuttingDown");
        assert_eq!(format!("{:?}", ShutdownState::Draining), "Draining");
        assert_eq!(format!("{:?}", ShutdownState::Stopped), "Stopped");
        assert_eq!(
            format!("{:?}", ShutdownState::ForcefulShuttingDown),
            "ForcefulShuttingDown"
        );
    }

    #[test]
    fn test_shutdown_mode_debug() {
        assert_eq!(format!("{:?}", ShutdownMode::Graceful), "Graceful");
        assert_eq!(format!("{:?}", ShutdownMode::Forceful), "Forceful");
    }

    #[test]
    fn test_drain_poll_interval_test_mode() {
        // In test mode, drain_poll_interval should return 100ms (not 2s)
        assert_eq!(drain_poll_interval(), std::time::Duration::from_millis(100));
    }

    #[test]
    fn test_busy_count_unchanged_in_forceful_mode() {
        let handle = ShutdownHandle::new();

        // Start a graceful shutdown with pending work
        handle.coordinator.try_start_shutdown();
        handle.increment_busy();
        assert_eq!(handle.busy_count(), 1);

        // Escalate to forceful
        assert!(handle.escalate_to_forceful());
        assert!(handle.is_forceful());

        // busy_count is still 1 — forceful mode doesn't clear it;
        // the drain path simply skips waiting for it to reach 0
        assert_eq!(handle.busy_count(), 1);

        // Decrement still works normally
        handle.decrement_busy();
        assert_eq!(handle.busy_count(), 0);
    }

    #[tokio::test]
    async fn test_subscribe_drain_triggers_on_escalation() {
        let handle = ShutdownHandle::new();
        let mut rx = handle.subscribe_drain();
        handle.increment_busy();

        // Spawn initiate_shutdown — it will block on drain because busy_count > 0
        let h = handle.clone();
        tokio::spawn(async move {
            h.initiate_shutdown().await;
        });
        // Let it enter the drain loop
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(handle.is_shutting_down());

        // Escalate — drain_done_tx fires during initiate_shutdown,
        // so the subscriber should receive the signal
        handle.escalate_to_forceful();
        // Release busy count so drain can finalize
        handle.decrement_busy();

        // Wait for shutdown to finish
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // The subscriber received at least one drain signal
        // (sent in initiate_shutdown before the drain loop)
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_handle_escalate_idempotent() {
        let handle = ShutdownHandle::new();
        handle.coordinator.try_start_shutdown();

        // First escalation succeeds
        assert!(handle.escalate_to_forceful());
        assert!(handle.is_forceful());
        assert_eq!(handle.mode(), ShutdownMode::Forceful);

        // Second escalation is a no-op (already forceful)
        assert!(!handle.escalate_to_forceful());
        assert!(handle.is_forceful());
    }

    #[test]
    fn test_is_shutting_down_true_when_draining() {
        let handle = ShutdownHandle::new();
        handle.coordinator.try_start_shutdown();
        handle.coordinator.start_drain();

        // Draining is still "shutting down" — components should reject new work
        assert!(handle.is_shutting_down());
        assert!(!handle.is_forceful());
    }

    #[test]
    fn test_is_shutting_down_false_when_stopped() {
        let handle = ShutdownHandle::new();
        handle.coordinator.try_start_shutdown();
        handle.coordinator.start_drain();
        handle.coordinator.mark_stopped();

        // Stopped is not "shutting down" — the shutdown is complete
        assert!(!handle.is_shutting_down());
        assert!(!handle.is_forceful());
    }

    #[tokio::test]
    async fn test_graceful_drain_timeout() {
        // After timeout, drain completes even if busy_count > 0.
        let handle =
            ShutdownHandle::new().with_drain_timeout(std::time::Duration::from_millis(100));
        // Register two pending operations — neither will complete
        handle.increment_busy();
        handle.increment_busy();
        assert_eq!(handle.busy_count(), 2);

        let h = handle.clone();
        let shutdown_handle = tokio::spawn(async move {
            h.initiate_shutdown().await;
        });

        // Wait for timeout to fire + buffer
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Shutdown should have completed despite busy_count > 0
        shutdown_handle.await.unwrap();
        assert!(handle.is_stopped(), "drain should complete after timeout");
        // busy_count was not cleared by the drain
        assert_eq!(handle.busy_count(), 2);
    }

    #[tokio::test]
    async fn test_drain_timeout_returns_remaining_count() {
        // Timeout leaves busy_count intact — caller gets the remaining count.
        let handle =
            ShutdownHandle::new().with_drain_timeout(std::time::Duration::from_millis(200));
        handle.increment_busy();
        handle.increment_busy();
        handle.increment_busy();
        assert_eq!(handle.busy_count(), 3);

        let h = handle.clone();
        tokio::spawn(async move {
            h.initiate_shutdown().await;
        });

        // Wait for timeout + buffer
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert!(handle.is_stopped());
        // busy_count still reflects the 3 in-flight operations
        assert_eq!(handle.busy_count(), 3);
    }

    #[tokio::test]
    async fn test_drain_completes_on_zero_count() {
        // When busy_count reaches 0, drain completes immediately
        // without waiting for the full timeout.
        let handle = ShutdownHandle::new().with_drain_timeout(std::time::Duration::from_secs(10));
        handle.increment_busy();
        handle.increment_busy();

        let h = handle.clone();
        let shutdown_handle = tokio::spawn(async move {
            h.initiate_shutdown().await;
        });

        // Let it enter the drain loop
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(!handle.is_stopped());

        // Complete both operations
        handle.decrement_busy();
        handle.decrement_busy();

        // Should complete quickly, not wait for 10s timeout
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), shutdown_handle).await;
        assert!(
            result.is_ok(),
            "drain should complete when busy_count hits 0"
        );
        assert!(handle.is_stopped());
        assert_eq!(handle.busy_count(), 0);
    }

    #[tokio::test]
    async fn test_forceful_skips_drain() {
        // Forceful mode terminates immediately, ignoring busy_count.
        let handle = ShutdownHandle::new();
        for _ in 0..50 {
            handle.increment_busy();
        }

        let h = handle.clone();
        let shutdown_handle = tokio::spawn(async move {
            h.initiate_shutdown().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(handle.is_shutting_down());
        assert!(!handle.is_stopped());

        // Escalate to forceful — should terminate immediately
        handle.escalate_to_forceful();
        shutdown_handle.await.unwrap();

        assert!(handle.is_stopped());
        // busy_count unchanged — forceful skips drain
        assert_eq!(handle.busy_count(), 50);
    }
}
