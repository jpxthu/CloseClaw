//! GLM LLM Provider — pure HTTP transport for the
//! GLM Chat Completions API.

mod models;
pub mod plugin;
mod provider;
mod quota;
mod streaming;
mod types;

pub use plugin::GlmPlugin;
pub use provider::GlmProvider;

#[allow(unused_imports)]
pub(crate) use crate::types::RawSseChunk;
#[allow(unused_imports)]
pub(crate) use crate::{ModelInfo, ModelLister};
#[allow(unused_imports)]
pub(crate) use types::*;

#[cfg(test)]
mod streaming_tests;
#[cfg(test)]
mod tests;
