//! LLM Protocol implementations — re-exports and module declarations.
//!
//! The [`ChatProtocol`] trait lives in [`chat_protocol`]; concrete protocol
//! implementations are in the sibling modules.

mod chat_protocol;
// Placeholder stubs — real implementations added in subsequent steps
pub mod anthropic;
pub mod glm;
pub mod openai;

// Re-export trait + types from chat_protocol
pub use chat_protocol::{
    ChatProtocol, IncomingSseStream, OutgoingEventStream, ProtocolError, Result,
};

// Re-export concrete protocol implementations
pub use anthropic::AnthropicProtocol;
pub use glm::GlmProtocol;
pub use openai::OpenAiProtocol;
