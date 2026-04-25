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
//!     .then_err(LLMError::RateLimitExceeded)
//!     .build();
//! ```

use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{ChatRequest, ChatResponse, LLMError, LLMProvider, Message, Usage};

/// A scenario defines what the next `chat()` call should return.
#[derive(Debug, Clone)]
pub enum Scenario {
    /// Respond with a successful chat response.
    Ok {
        content: String,
        model: String,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// Respond with an error.
    Err(LLMError),
    /// Sleep for the given duration then behave as the wrapped scenario.
    Delay {
        duration: Duration,
        inner: Box<Scenario>,
    },
}

impl Scenario {
    /// Shortcut: a successful scenario with default usage metrics.
    pub fn ok(content: impl Into<String>, model: impl Into<String>) -> Self {
        Self::Ok {
            content: content.into(),
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: 10,
        }
    }

    /// Shortcut: an error scenario.
    pub fn err(error: LLMError) -> Self {
        Self::Err(error)
    }

    fn usage(&self) -> Usage {
        match self {
            Self::Ok {
                prompt_tokens,
                completion_tokens,
                ..
            } => Usage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                total_tokens: *prompt_tokens + *completion_tokens,
            },
            Self::Err(_) => Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
            },
            Self::Delay { inner, .. } => inner.usage(),
        }
    }

    fn content(&self) -> String {
        match self {
            Self::Ok { content, .. } => content.clone(),
            Self::Err(_) => String::new(),
            Self::Delay { inner, .. } => inner.content(),
        }
    }

    fn model(&self) -> String {
        match self {
            Self::Ok { model, .. } => model.clone(),
            Self::Err(_) => String::new(),
            Self::Delay { inner, .. } => inner.model(),
        }
    }
}

/// Captures an incoming `ChatRequest` for test inspection.
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
struct SharedState {
    /// Queue of scenarios to consume in FIFO order.
    scenarios: VecDeque<Scenario>,
    /// Captured requests for test inspection.
    captured: Vec<CapturedRequest>,
    /// Panic on scenario exhaustion (unless a fallback is set).
    panic_on_exhaust: bool,
    /// Fallback response when scenarios are exhausted (set via `.or_else()`).
    fallback: Option<String>,
    /// Fallback model for or_else fallback.
    fallback_model: String,
    /// Whether is_stub returns true.
    stub_flag: bool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            scenarios: VecDeque::new(),
            captured: Vec::new(),
            panic_on_exhaust: true,
            fallback: None,
            fallback_model: "fake-model".to_string(),
            stub_flag: true,
        }
    }
}

/// A scenario-driven fake LLM provider for testing.
///
/// Each `chat()` call consumes the next scenario from the queue.
/// When the queue is empty the provider panics (by default) or returns a
/// configured fallback response.
///
/// # Builder
///
/// ```
/// # use closeclaw::llm::{LLMProvider, FakeProvider, ChatRequest, Message};
/// let provider = FakeProvider::builder()
///     .then_ok("response 1", "gpt-4")
///     .then_err(closeclaw::llm::LLMError::RateLimitExceeded)
///     .stub(true)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct FakeProvider {
    inner: Arc<Mutex<SharedState>>,
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

    /// Returns all captured requests without consuming them.
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
        self.inner.lock().unwrap().captured.clear();
    }

    fn next_scenario(&self) -> Option<Scenario> {
        self.inner.lock().unwrap().scenarios.pop_front()
    }

    fn capture(&self, req: ChatRequest) {
        self.inner.lock().unwrap().captured.push(req.into());
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
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for `FakeProvider`.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    state: SharedState,
}

impl Builder {
    /// Add a successful scenario — consumes the next call.
    pub fn then_ok(mut self, content: impl Into<String>, model: impl Into<String>) -> Self {
        self.state.scenarios.push_back(Scenario::ok(content, model));
        self
    }

    /// Add a successful scenario with custom usage — consumes the next call.
    pub fn then_ok_with(
        mut self,
        content: impl Into<String>,
        model: impl Into<String>,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Ok {
            content: content.into(),
            model: model.into(),
            prompt_tokens,
            completion_tokens,
        });
        self
    }

    /// Add an error scenario — consumes the next call.
    pub fn then_err(mut self, error: LLMError) -> Self {
        self.state.scenarios.push_back(Scenario::err(error));
        self
    }

    /// Add an error scenario with a custom message — consumes the next call.
    pub fn then_err_with(mut self, error: LLMError) -> Self {
        self.state.scenarios.push_back(Scenario::err(error));
        self
    }

    /// After all scenarios are exhausted, return this fallback content instead of panicking.
    pub fn or_else(mut self, content: impl Into<String>) -> Self {
        self.state.panic_on_exhaust = false;
        self.state.fallback = Some(content.into());
        self
    }

    /// Configure the stub flag returned by `is_stub()`.
    pub fn stub(mut self, val: bool) -> Self {
        self.state.stub_flag = val;
        self
    }

    /// Build the `FakeProvider`.
    pub fn build(self) -> FakeProvider {
        FakeProvider {
            inner: Arc::new(Mutex::new(self.state)),
        }
    }
}

#[async_trait]
impl LLMProvider for FakeProvider {
    fn name(&self) -> &str {
        "fake"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        self.capture(request.clone());

        let mut current = match self.next_scenario() {
            Some(s) => s,
            None => {
                let state = self.inner.lock().unwrap();
                if state.panic_on_exhaust {
                    drop(state);
                    panic!("{}", self.exhausted_message());
                }
                return Ok(ChatResponse {
                    content: state.fallback.clone().unwrap_or_default(),
                    model: state.fallback_model.clone(),
                    usage: Usage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                    },
                });
            }
        };

        loop {
            match current {
                Scenario::Delay { duration, inner } => {
                    tokio::time::sleep(duration).await;
                    current = *inner;
                }
                Scenario::Ok { .. } => {
                    let usage = current.usage();
                    let content = current.content();
                    let model = current.model();
                    return Ok(ChatResponse {
                        content,
                        model,
                        usage,
                    });
                }
                Scenario::Err(err) => return Err(err),
            }
        }
    }

    fn models(&self) -> Vec<&'static str> {
        vec!["fake-model"]
    }

    fn is_stub(&self) -> bool {
        self.inner.lock().unwrap().stub_flag
    }
}
