//! Shutdown signal abstraction.
//!
//! Provides [`ShutdownSignal`], a trait that decouples the LLM layer
//! from the concrete `ShutdownHandle` type. The daemon's
//! `ShutdownHandle` (in the main crate) implements this trait; LLM
//! code depends only on the trait object.

/// Shutdown state machine.
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
    /// Convert from raw `u8` stored in an `AtomicU8`.
    pub fn from_u8(v: u8) -> Self {
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
    pub fn is_shutting_down_state(self) -> bool {
        matches!(
            self,
            ShutdownState::ShuttingDown
                | ShutdownState::Draining
                | ShutdownState::ForcefulShuttingDown
        )
    }

    /// Returns the shutdown mode for an active shutdown state.
    pub fn mode(self) -> ShutdownMode {
        match self {
            ShutdownState::ForcefulShuttingDown => ShutdownMode::Forceful,
            _ => ShutdownMode::Graceful,
        }
    }
}

/// Structured drain status returned by [`ShutdownSignal::drain_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainStatus {
    /// Current shutdown state.
    pub state: ShutdownState,
    /// Number of in-flight operations.
    pub busy_count: usize,
    /// Whether the coordinator is actively draining.
    pub is_draining: bool,
}

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

    /// Returns a structured snapshot of the current drain status.
    fn drain_status(&self) -> DrainStatus;
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

    fn drain_status(&self) -> DrainStatus {
        self.signal.drain_status()
    }
}
