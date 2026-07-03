//! Shutdown signal abstraction.
//!
//! Provides [`ShutdownSignal`], a trait that decouples the LLM layer
//! from the concrete `ShutdownHandle` type. The gateway's
//! `ShutdownHandle` (in the gateway crate) implements this trait; LLM
//! code depends only on the trait object.
//!
//! This module only contains types and trait definitions — no executable logic.

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

/// Abstract shutdown signal — decouples the LLM crate from the gateway's
/// concrete `ShutdownHandle`.
///
/// The LLM layer uses `Option<Arc<dyn ShutdownSignal>>` for busy-count
/// tracking during tool execution. The gateway's `ShutdownHandle`
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
