//! Shutdown signal abstraction.
//!
//! Provides [`ShutdownSignal`], a trait that decouples the LLM layer
//! from the concrete `ShutdownHandle` type. The daemon's
//! `ShutdownHandle` (in the main crate) implements this trait; LLM
//! code depends only on the trait object.

/// Shutdown mode — distinguishes graceful from forceful shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShutdownMode {
    /// Graceful: wait for in-flight operations to complete
    #[default]
    Graceful,
    /// Forceful: immediately terminate operations
    Forceful,
}

/// Abstract shutdown signal — decouples the LLM crate from the daemon's
/// concrete `ShutdownHandle`.
///
/// The LLM layer uses `Option<Arc<dyn ShutdownSignal>>` for busy-count
/// tracking during tool execution. The daemon's `ShutdownHandle`
/// implements this trait.
pub trait ShutdownSignal: Send + Sync {
    /// Returns `true` if a shutdown has been initiated.
    fn is_shutting_down(&self) -> bool;

    /// Increment the busy count before starting async work.
    fn increment_busy(&self);

    /// Decrement the busy count after async work completes.
    fn decrement_busy(&self);

    /// Get the current busy count.
    fn busy_count(&self) -> usize;

    /// Atomically escalate from graceful to forceful shutdown.
    /// Returns true if escalation succeeded, false if already escalated.
    fn escalate_to_forceful(&self) -> bool;

    /// Returns true if forceful shutdown has been escalated.
    fn is_forceful(&self) -> bool;
}

/// Shared shutdown handle with busy-count tracking.
///
/// Components hold an `Arc<ShutdownHandle>` to cooperate with shutdown.
/// The handle tracks busy-count and delegates state checks to the
/// underlying `ShutdownSignal` trait object.
pub struct ShutdownHandle {
    /// The underlying shutdown signal (delegates to daemon's ShutdownCoordinator).
    signal: std::sync::Arc<dyn ShutdownSignal>,
}

impl ShutdownHandle {
    /// Create a new handle wrapping the given shutdown signal.
    pub fn new(signal: std::sync::Arc<dyn ShutdownSignal>) -> Self {
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
            signal: std::sync::Arc::clone(&self.signal),
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
}
