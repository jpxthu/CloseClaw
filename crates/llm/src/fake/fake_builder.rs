//! Builder for `FakeProvider`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::fake_scenario::{DeliveryConfig, ErrorInjection, StreamInterrupt};
use super::{FakeProvider, Scenario, SharedState};
use crate::provider::ProviderError;
use crate::types::{ProtocolId, RawContentBlock};

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
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens,
            completion_tokens,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: super::fake_scenario::DeliveryConfig::default(),
            include_usage: false,
            protocol: crate::types::ProtocolId::new("openai"),
            segment_granularity: 0,
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
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens,
            completion_tokens,
            cache_read_tokens: cache.0,
            cache_write_tokens: cache.1,
            delivery: super::fake_scenario::DeliveryConfig::default(),
            include_usage: false,
            protocol: crate::types::ProtocolId::new("openai"),
            segment_granularity: 0,
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

    /// Add a streaming scenario with custom protocol and segment granularity.
    pub fn then_streaming(
        mut self,
        content: impl Into<String>,
        model: impl Into<String>,
        protocol: ProtocolId,
        granularity: usize,
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Ok {
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: 10,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: DeliveryConfig::default(),
            include_usage: false,
            protocol,
            segment_granularity: granularity,
        });
        self
    }

    /// Add a scenario with first-token delay.
    pub fn then_with_first_token_delay(
        mut self,
        content: impl Into<String>,
        model: impl Into<String>,
        delay: Duration,
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Ok {
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: 10,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: DeliveryConfig {
                first_token_delay: Some(delay),
                ..Default::default()
            },
            include_usage: false,
            protocol: ProtocolId::new("openai"),
            segment_granularity: 0,
        });
        self
    }

    /// Add a scenario with per-segment delay.
    pub fn then_with_per_segment_delay(
        mut self,
        content: impl Into<String>,
        model: impl Into<String>,
        delay: Duration,
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Ok {
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: 10,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: DeliveryConfig {
                per_segment_delay: Some(delay),
                ..Default::default()
            },
            include_usage: false,
            protocol: ProtocolId::new("openai"),
            segment_granularity: 0,
        });
        self
    }

    /// Add a scenario with overall delay (non-streaming only).
    pub fn then_with_overall_delay(
        mut self,
        content: impl Into<String>,
        model: impl Into<String>,
        delay: Duration,
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Ok {
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: 10,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: DeliveryConfig {
                overall_delay: Some(delay),
                ..Default::default()
            },
            include_usage: false,
            protocol: ProtocolId::new("openai"),
            segment_granularity: 0,
        });
        self
    }

    /// Add a scenario that injects an HTTP error.
    pub fn then_http_error(
        mut self,
        content: impl Into<String>,
        model: impl Into<String>,
        status_code: u16,
        retry_after: Option<u64>,
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Ok {
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: DeliveryConfig {
                error_injection: Some(ErrorInjection {
                    status_code,
                    message: format!("HTTP {}", status_code),
                    retry_after,
                }),
                ..Default::default()
            },
            include_usage: false,
            protocol: ProtocolId::new("openai"),
            segment_granularity: 0,
        });
        self
    }

    /// Add a scenario that interrupts the stream after N frames.
    pub fn then_stream_interrupt(
        mut self,
        content: impl Into<String>,
        model: impl Into<String>,
        interrupt_after_frames: usize,
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Ok {
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: 10,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: DeliveryConfig {
                stream_interrupt: Some(StreamInterrupt {
                    interrupt_after_frames,
                }),
                ..Default::default()
            },
            include_usage: false,
            protocol: ProtocolId::new("openai"),
            segment_granularity: 0,
        });
        self
    }

    /// Add a thinking (reasoning) content block to the last scenario.
    pub fn then_thinking(mut self, thinking: impl Into<String>, signature: Option<String>) -> Self {
        if let Some(Scenario::Ok {
            ref mut content_blocks,
            ..
        }) = self.state.scenarios.back_mut()
        {
            content_blocks.push(RawContentBlock::Thinking {
                thinking: thinking.into(),
                signature,
            });
        }
        self
    }

    /// Add a tool use content block to the last scenario.
    pub fn then_tool_use(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        input: impl Into<String>,
    ) -> Self {
        if let Some(Scenario::Ok {
            ref mut content_blocks,
            ..
        }) = self.state.scenarios.back_mut()
        {
            content_blocks.push(RawContentBlock::ToolUse {
                id: id.into(),
                name: name.into(),
                input: input.into(),
            });
        }
        self
    }

    /// Add a model discovery scenario — returns the given models list.
    pub fn then_models(mut self, models: Vec<String>, model: impl Into<String>) -> Self {
        self.state.scenarios.push_back(Scenario::Models {
            models,
            model: model.into(),
            delivery: DeliveryConfig::default(),
        });
        self
    }

    /// Add a model discovery scenario that injects an HTTP error.
    pub fn then_models_error(
        mut self,
        model: impl Into<String>,
        status_code: u16,
        retry_after: Option<u64>,
    ) -> Self {
        self.state.scenarios.push_back(Scenario::Models {
            models: Vec::new(),
            model: model.into(),
            delivery: DeliveryConfig {
                error_injection: Some(ErrorInjection {
                    status_code,
                    message: format!("HTTP {}", status_code),
                    retry_after,
                }),
                ..Default::default()
            },
        });
        self
    }

    /// Set whether usage metrics are included in streaming responses.
    pub fn include_usage(mut self, val: bool) -> Self {
        if let Some(Scenario::Ok {
            ref mut include_usage,
            ..
        }) = self.state.scenarios.back_mut()
        {
            *include_usage = val;
        }
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
