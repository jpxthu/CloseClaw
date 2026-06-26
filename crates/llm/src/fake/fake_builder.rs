//! Builder for `FakeProvider`.

use std::sync::{Arc, Mutex};

use super::{FakeProvider, Scenario, SharedState};
use crate::provider::ProviderError;

/// Builder for `FakeProvider`.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    pub(crate) state: SharedState,
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
            cache_read_tokens: None,
            cache_write_tokens: None,
        });
        self
    }

    /// Add a successful scenario with custom usage and cache metrics — consumes the next call.
    pub fn then_ok_with_cache(
        mut self,
        content: impl Into<String>,
        model: impl Into<String>,
        prompt_tokens: u32,
        completion_tokens: u32,
        cache: (Option<u32>, Option<u32>),
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Ok {
            content: content.into(),
            model: model.into(),
            prompt_tokens,
            completion_tokens,
            cache_read_tokens: cache.0,
            cache_write_tokens: cache.1,
        });
        self
    }

    /// Add an error scenario — consumes the next call.
    pub fn then_err(mut self, error: ProviderError) -> Self {
        self.state.scenarios.push_back(Scenario::err(error));
        self
    }

    /// Add an error scenario with custom usage metrics — consumes the next call.
    pub fn then_err_with(
        mut self,
        error: ProviderError,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> Self {
        self.state
            .scenarios
            .push_back(Scenario::err_with(error, prompt_tokens, completion_tokens));
        self
    }

    /// Add a delay scenario — sleeps for `duration` then resolves as `inner`.
    pub fn then_delay(mut self, duration: std::time::Duration, inner: Scenario) -> Self {
        self.state
            .scenarios
            .push_back(Scenario::delay(duration, inner));
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
