//! Shutdown handle — wraps a `dyn ShutdownSignal` to provide a
//! concrete type that components can hold as `Arc<ShutdownHandle>`.
//!
//! This type delegates all calls to the inner signal object.
//! It was migrated from `closeclaw_common::shutdown` in Step 1.2.

use std::sync::Arc;

use closeclaw_common::shutdown::{DrainStatus, ShutdownSignal};

/// Shared shutdown handle with busy-count tracking.
///
/// Components hold an `Arc<ShutdownHandle>` to cooperate with shutdown.
/// The handle tracks busy-count and delegates state checks to the
/// underlying `ShutdownSignal` trait object.
pub struct ShutdownHandle {
    /// The underlying shutdown signal (delegates to daemon's ShutdownCoordinator).
    signal: Arc<dyn ShutdownSignal>,
}

impl ShutdownHandle {
    /// Create a new handle wrapping the given shutdown signal.
    pub fn new(signal: Arc<dyn ShutdownSignal>) -> Self {
        Self { signal }
    }

    /// Returns `true` if shutdown has been initiated.
    pub fn is_shutting_down(&self) -> bool {
        self.signal.is_shutting_down()
    }

    /// Increment the busy count.
    pub fn increment_busy(&self) {
        self.signal.increment_busy();
    }

    /// Decrement the busy count.
    pub fn decrement_busy(&self) {
        self.signal.decrement_busy();
    }

    /// Get the current busy count.
    pub fn busy_count(&self) -> usize {
        self.signal.busy_count()
    }

    /// Atomically escalate from graceful to forceful shutdown.
    /// Returns true if escalation succeeded, false if already escalated.
    pub fn escalate_to_forceful(&self) -> bool {
        self.signal.escalate_to_forceful()
    }

    /// Returns true if forceful shutdown has been escalated.
    pub fn is_forceful(&self) -> bool {
        self.signal.is_forceful()
    }

    /// Returns a structured snapshot of the current drain status.
    pub fn drain_status(&self) -> DrainStatus {
        self.signal.drain_status()
    }
}

impl std::fmt::Debug for ShutdownHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownHandle")
            .field("busy_count", &self.busy_count())
            .finish_non_exhaustive()
    }
}

impl Clone for ShutdownHandle {
    fn clone(&self) -> Self {
        Self {
            signal: Arc::clone(&self.signal),
        }
    }
}

impl ShutdownSignal for ShutdownHandle {
    fn is_shutting_down(&self) -> bool {
        self.signal.is_shutting_down()
    }

    fn increment_busy(&self) {
        self.signal.increment_busy();
    }

    fn decrement_busy(&self) {
        self.signal.decrement_busy();
    }

    fn busy_count(&self) -> usize {
        self.signal.busy_count()
    }

    fn escalate_to_forceful(&self) -> bool {
        self.signal.escalate_to_forceful()
    }

    fn is_forceful(&self) -> bool {
        self.signal.is_forceful()
    }

    fn drain_status(&self) -> DrainStatus {
        self.signal.drain_status()
    }
}
