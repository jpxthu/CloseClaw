//! Shutdown signal abstraction.
//!
//! Provides [`ShutdownSignal`], a trait that decouples the LLM layer
//! from the concrete `ShutdownHandle` type. The daemon's
//! `ShutdownHandle` (in the main crate) implements this trait; LLM
//! code depends only on the trait object.

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
}
