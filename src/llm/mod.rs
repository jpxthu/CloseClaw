//! LLM Interface — provider abstraction and chat types
//!
//! Re-exported from `closeclaw-llm` crate.

pub use closeclaw_llm::*;

#[cfg(feature = "fake-llm")]
pub use closeclaw_llm::fake;
#[cfg(feature = "fake-llm")]
pub use closeclaw_llm::FakeProvider;
