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

/// The actually-used LLM call target of one round.
///
/// Reported by the production caller after a successful call (the
/// fallback chain knows which entry actually served the request);
/// consumed by the gateway to distinguish model/provider switches for
/// the cache hit consistency check. Test fakes that do not report a
/// target leave it `None`, and consumers degrade to session-config
/// semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallTarget {
    /// Identifier of the provider that actually served the call.
    pub provider_id: String,
    /// Identifier of the model that actually served the call.
    pub model_id: String,
}

impl CallTarget {
    /// Combined `provider/model` identifier string.
    ///
    /// This is the model-switch exemption key stored in
    /// [`RunningStats::last_call_model`](crate::llm_stats::RunningStats::last_call_model).
    pub fn id_string(&self) -> String {
        format!("{}/{}", self.provider_id, self.model_id)
    }
}

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

    /// Returns whether the underlying provider supports parallel tool calls.
    ///
    /// When `false`, the framework must serialize all tool calls for this
    /// provider, regardless of the agent-level `parallel_tool_calls` setting.
    /// Default is `false` (conservative).
    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }

    /// Returns the actually-used [`CallTarget`] (provider/model) of the
    /// most recent successful call, if the implementation can report it.
    ///
    /// Production fallback-backed implementations record the chain entry
    /// that actually served the last successful call (streaming,
    /// non-streaming, and degraded paths alike). The default returns
    /// `None` so test fakes and simple callers need no changes; callers
    /// of this method must degrade to session-config semantics on `None`.
    fn last_call_target(&self) -> Option<CallTarget> {
        None
    }
}
