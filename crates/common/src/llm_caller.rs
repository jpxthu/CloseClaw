//! Abstract LLM caller trait.
//!
//! Defines the [`LlmCaller`] interface for making LLM requests. This trait
//! lives in `closeclaw-common` (Layer 0) so that both `closeclaw-session`
//! (implementation) and `closeclaw-gateway` (consumer) can reference it
//! without creating cross-layer dependencies.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

use crate::llm_error::LLMError;
use crate::llm_types::InternalRequest;
use crate::processor::{StreamEvent, UnifiedResponse};

/// Abstract interface for making LLM requests.
///
/// Implementations live in the session layer (e.g., wrapping
/// `UnifiedFallbackClient` or `UnifiedChatClient`). Consumers in higher
/// layers (gateway, daemon) depend only on this trait.
#[async_trait]
pub trait LlmCaller: Send + Sync {
    /// Make a non-streaming LLM call.
    async fn call(&self, request: InternalRequest) -> Result<UnifiedResponse, LLMError>;

    /// Make a streaming LLM call.
    ///
    /// Returns a pinned stream of [`StreamEvent`] items. Each item is either
    /// a successful event or an [`LLMError`].
    async fn call_streaming(
        &self,
        request: InternalRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>, LLMError>;

    /// Returns the provider's default header key-value pairs.
    ///
    /// Used for fingerprinting prompt components to detect cache breaks.
    /// Sensitive headers (e.g. `Authorization`, `api-key`) have their values
    /// replaced with a stable placeholder to avoid leaking credentials.
    ///
    /// Default implementation returns an empty `Vec`.
    fn default_header_pairs(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}
