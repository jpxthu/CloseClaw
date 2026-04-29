//! Core trait and phase enumeration for message processors.

use super::context::{MessageContext, ProcessedMessage};
use super::error::ProcessError;
use async_trait::async_trait;

/// Processing phase — determines which chain a processor belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessPhase {
    /// Inbound — processes incoming messages before LLM.
    Inbound,
    /// Outbound — processes LLM output before sending.
    Outbound,
}

/// Message processor interface.
///
/// Implementations must be `Send + Sync + 'static`.
#[async_trait]
pub trait MessageProcessor: Send + Sync {
    /// Returns the unique name of this processor.
    fn name(&self) -> &str;

    /// Returns the processing phase this processor belongs to.
    fn phase(&self) -> ProcessPhase;

    /// Returns the priority for this processor.
    ///
    /// Lower values run first within a phase chain.
    fn priority(&self) -> u8;

    /// Process the given message context.
    ///
    /// Returns `Some(ProcessedMessage)` when processing succeeded
    /// and the result should be passed to the next processor.
    /// Returns `None` when this processor chooses to skip.
    async fn process(&self, ctx: &MessageContext)
        -> Result<Option<ProcessedMessage>, ProcessError>;
}
