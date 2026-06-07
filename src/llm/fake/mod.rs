//! Fake LLM Provider — scenario-driven fake responses for testing
//!
//! Provides a fully configurable fake LLM provider for E2E and integration tests.
//! Scenarios are consumed in FIFO order; when exhausted the provider panics by default.
//! Use the Builder API (`.then_ok()`, `.then_err()`, etc.) to set up scenarios.
//!
//! # Example
//!
//! ```ignore
//! let provider = FakeProvider::builder()
//!     .then_ok("hello", "model-x")
//!     .then_err(ProviderError::Legacy("rate limit".into()))
//!     .build();
//! ```

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;

use super::provider::{Provider, ProviderError, SseStream};
use super::types::{
    InternalRequest, InternalResponse, ProtocolId, RawContentBlock, RawSseChunk, RawUsage,
};
use super::{ChatRequest, Message};

pub mod fake_builder;
pub mod fake_scenario;
pub use fake_builder::Builder;
pub use fake_scenario::Scenario;

/// Captures an incoming [`InternalRequest`] for test inspection.
#[derive(Debug, Clone)]
pub struct CapturedInternalRequest {
    pub request: InternalRequest,
}

/// Captures an incoming [`ChatRequest`] for test inspection (legacy compat).
#[derive(Debug, Clone)]
pub struct CapturedRequest {
    pub model: String,
    pub messages: Vec<Message>,
}

impl From<ChatRequest> for CapturedRequest {
    fn from(req: ChatRequest) -> Self {
        Self {
            model: req.model,
            messages: req.messages,
        }
    }
}

/// Internal state shared across clones of FakeProvider.
#[derive(Debug, Clone)]
pub struct SharedState {
    /// Queue of scenarios to consume in FIFO order.
    pub scenarios: VecDeque<Scenario>,
    /// Captured InternalRequests for test inspection.
    pub captured_internal: Vec<CapturedInternalRequest>,
    /// Captured ChatRequests for test inspection (legacy compat).
    pub captured: Vec<CapturedRequest>,
    /// Panic on scenario exhaustion (unless a fallback is set).
    pub panic_on_exhaust: bool,
    /// Fallback response when scenarios are exhausted (set via `.or_else()`).
    pub fallback: Option<String>,
    /// Fallback model for or_else fallback.
    pub fallback_model: String,
    /// Whether is_stub returns true.
    pub stub_flag: bool,
    /// HTTP client for Provider trait.
    pub http_client: Client,
    /// Default headers for Provider trait.
    pub default_headers: HeaderMap,
    /// Supported protocol IDs for Provider trait.
    pub supported_protocols: Vec<ProtocolId>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            scenarios: VecDeque::new(),
            captured_internal: Vec::new(),
            captured: Vec::new(),
            panic_on_exhaust: true,
            fallback: None,
            fallback_model: "fake-model".to_string(),
            stub_flag: true,
            http_client: Client::new(),
            default_headers: HeaderMap::new(),
            supported_protocols: vec![ProtocolId::new("openai")],
        }
    }
}

/// A scenario-driven fake LLM provider for testing.
///
/// Each `chat()` / `send()` call consumes the next scenario from the queue.
/// When the queue is empty the provider panics (by default) or returns a
/// configured fallback response.
///
/// # Builder
///
/// ```
/// # use closeclaw::llm::{FakeProvider, ChatRequest, Message};
/// # use closeclaw::llm::provider::ProviderError;
/// let provider = FakeProvider::builder()
///     .then_ok("response 1", "gpt-4")
///     .then_err(ProviderError::Legacy("rate limit".into()))
///     .stub(true)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct FakeProvider {
    pub inner: Arc<Mutex<SharedState>>,
}

impl FakeProvider {
    /// Start building a FakeProvider.
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// Returns a new FakeProvider with no scenarios (always panics on first call).
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SharedState::default())),
        }
    }

    /// Returns all captured InternalRequests without consuming them.
    pub fn captured_internal_requests(&self) -> Vec<CapturedInternalRequest> {
        self.inner.lock().unwrap().captured_internal.clone()
    }

    /// Returns all captured ChatRequests without consuming them (legacy compat).
    pub fn captured_requests(&self) -> Vec<CapturedRequest> {
        self.inner.lock().unwrap().captured.clone()
    }

    /// Removes and returns all captured requests.
    pub fn drain_requests(&mut self) -> Vec<CapturedRequest> {
        let mut state = self.inner.lock().unwrap();
        std::mem::take(&mut state.captured)
    }

    /// Clears all captured requests.
    pub fn clear_requests(&mut self) {
        let mut state = self.inner.lock().unwrap();
        state.captured.clear();
        state.captured_internal.clear();
    }

    /// Returns whether this provider is a stub.
    pub fn is_stub(&self) -> bool {
        self.inner.lock().unwrap().stub_flag
    }

    fn next_scenario(&self) -> Option<Scenario> {
        self.inner.lock().unwrap().scenarios.pop_front()
    }

    fn capture_internal(&self, req: &InternalRequest) {
        self.inner
            .lock()
            .unwrap()
            .captured_internal
            .push(CapturedInternalRequest {
                request: req.clone(),
            });
    }

    fn exhausted_message(&self) -> String {
        let state = self.inner.lock().unwrap();
        let captured = &state.captured;
        format!(
            "[FakeProvider] scenarios exhausted. captured_requests={} (last 5: {:?})",
            captured.len(),
            captured.iter().rev().take(5).collect::<Vec<_>>()
        )
    }

    /// Resolve the next scenario, handling Delay loops and exhaustion.
    /// Returns `Ok(Some(scenario))` if a non-delay scenario is available,
    /// `Ok(None)` if fallback was used, or `Err(ProviderError)` on exhaustion.
    async fn resolve_scenario(&self) -> Result<Option<Scenario>, ProviderError> {
        let mut current = match self.next_scenario() {
            Some(s) => s,
            None => {
                let state = self.inner.lock().unwrap();
                if state.panic_on_exhaust {
                    drop(state);
                    panic!("{}", self.exhausted_message());
                }
                // Return fallback — caller should use SharedState fallback
                return Ok(None);
            }
        };

        loop {
            match current {
                Scenario::Delay { duration, inner } => {
                    tokio::time::sleep(duration).await;
                    current = *inner;
                }
                other => return Ok(Some(other)),
            }
        }
    }
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new()
    }
}

// Tests live in fake_tests.rs but are compiled as part of this module.
// This keeps fake.rs under the 500-line pre-commit limit while preserving
// the original `use super::*` import semantics for the tests.
#[cfg(all(test, feature = "fake-llm"))]
#[path = "fake_tests.rs"]
mod tests;

// ── Provider trait impl ─────────────────────────────────────────────────────

#[async_trait]
impl Provider for FakeProvider {
    fn id(&self) -> &str {
        "fake"
    }

    fn base_url(&self) -> &str {
        ""
    }

    fn api_key(&self) -> &str {
        ""
    }

    fn supported_protocols(&self) -> &[ProtocolId] {
        let state = self.inner.lock().unwrap();
        drop(state);
        static PROTOCOLS: OnceLock<Vec<ProtocolId>> = OnceLock::new();
        let v = PROTOCOLS.get_or_init(|| vec![ProtocolId::new("openai")]);
        v.as_slice()
    }

    fn http_client(&self) -> &Client {
        static CLIENT: OnceLock<Client> = OnceLock::new();
        CLIENT.get_or_init(Client::new)
    }

    fn default_headers(&self) -> &HeaderMap {
        static EMPTY: OnceLock<HeaderMap> = OnceLock::new();
        EMPTY.get_or_init(HeaderMap::new)
    }

    async fn send(
        &self,
        request: InternalRequest,
        _body: serde_json::Value,
    ) -> super::provider::Result<InternalResponse> {
        self.capture_internal(&request);

        match self.resolve_scenario().await? {
            Some(scenario) => {
                let content = scenario.content();
                let usage = scenario.raw_usage();
                match scenario {
                    Scenario::Err { error, .. } => Err(error),
                    _ => Ok(InternalResponse {
                        content_blocks: vec![RawContentBlock::Text(content)],
                        usage,
                        finish_reason: None,
                    }),
                }
            }
            None => {
                let state = self.inner.lock().unwrap();
                let fallback = state.fallback.clone().unwrap_or_default();
                Ok(InternalResponse {
                    content_blocks: vec![RawContentBlock::Text(fallback)],
                    usage: RawUsage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: Some(0),
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                    },
                    finish_reason: None,
                })
            }
        }
    }

    async fn send_streaming(
        &self,
        request: InternalRequest,
        _body: serde_json::Value,
    ) -> super::provider::Result<SseStream> {
        self.capture_internal(&request);

        let (tx, rx) = mpsc::channel(32);

        match self.resolve_scenario().await? {
            Some(scenario) => match scenario {
                Scenario::Err { error, .. } => return Err(error),
                other => {
                    let content = other.content();

                    let _ = tx
                        .send(RawSseChunk {
                            event_type: "message".into(),
                            data: content,
                        })
                        .await;

                    let done = serde_json::json!({"type": "message_end"});
                    let _ = tx
                        .send(RawSseChunk {
                            event_type: "message".into(),
                            data: done.to_string(),
                        })
                        .await;
                }
            },
            None => {
                let fallback = {
                    let state = self.inner.lock().unwrap();
                    state.fallback.clone().unwrap_or_default()
                };
                let _ = tx
                    .send(RawSseChunk {
                        event_type: "message".into(),
                        data: fallback,
                    })
                    .await;
                let done = serde_json::json!({"type": "message_end"});
                let _ = tx
                    .send(RawSseChunk {
                        event_type: "message".into(),
                        data: done.to_string(),
                    })
                    .await;
            }
        }

        Ok(rx)
    }
}
