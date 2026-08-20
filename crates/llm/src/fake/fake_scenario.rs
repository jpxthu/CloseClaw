//! Scenario definitions for `FakeProvider`.

use std::time::Duration;

use crate::provider::ProviderError;
use crate::types::{ProtocolId, RawContentBlock, RawUsage};

/// HTTP error injection configuration.
///
/// When configured, the fake provider returns an HTTP error response instead
/// of a normal response. The `retry_after` field maps to the `Retry-After`
/// header value (in seconds).
#[derive(Debug, Clone)]
pub(crate) struct ErrorInjection {
    /// HTTP status code (e.g., 401, 429, 500).
    pub(crate) status_code: u16,
    /// Error response body message.
    pub(crate) message: String,
    /// Optional `Retry-After` header value in seconds.
    pub(crate) retry_after: Option<u64>,
}

/// Stream interrupt configuration.
///
/// When configured, the fake provider emits `interrupt_after_frames` SSE
/// frames and then abruptly closes the stream without sending a completion
/// event. This simulates a broken/incomplete streaming response.
#[derive(Debug, Clone)]
pub(crate) struct StreamInterrupt {
    /// Number of frames to emit before interrupting.
    pub(crate) interrupt_after_frames: usize,
}

/// Delivery configuration for `Scenario::Ok`.
///
/// Controls how the fake provider delivers responses: timing (delays),
/// error injection, streaming behavior, and protocol format.
#[derive(Debug, Clone, Default)]
pub(crate) struct DeliveryConfig {
    /// Delay before emitting the first SSE frame (streaming) or before
    /// returning the response (non-streaming). Simulates server processing
    /// time.
    pub(crate) first_token_delay: Option<Duration>,
    /// Delay between consecutive SSE frames during streaming. Simulates
    /// token-by-token generation pacing.
    pub(crate) per_segment_delay: Option<Duration>,
    /// Delay before returning the complete response (non-streaming only).
    /// Simulates overall server processing latency.
    pub(crate) overall_delay: Option<Duration>,
    /// HTTP error to inject. When set, the provider returns this error
    /// instead of a normal response.
    pub(crate) error_injection: Option<ErrorInjection>,
    /// Stream interruption. When set, the provider emits the specified
    /// number of frames and then closes the stream.
    pub(crate) stream_interrupt: Option<StreamInterrupt>,
}

/// A scenario defines what the next `chat()` / `send()` / `send_streaming()`
/// call should return.
#[derive(Debug)]
#[allow(private_interfaces)]
pub enum Scenario {
    /// Respond with a successful response, with optional delivery control.
    Ok {
        content_blocks: Vec<RawContentBlock>,
        model: String,
        prompt_tokens: u32,
        completion_tokens: u32,
        cache_read_tokens: Option<u32>,
        cache_write_tokens: Option<u32>,
        delivery: DeliveryConfig,
        /// Whether to include usage metrics in the streaming response.
        /// OpenAI only includes usage when `include_usage` is true.
        include_usage: bool,
        /// Protocol format for streaming responses.
        protocol: ProtocolId,
        /// Number of characters per segment for streaming. 0 = single chunk.
        segment_granularity: usize,
    },
    /// Respond with an error, optionally with usage metrics.
    Err {
        error: ProviderError,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// Sleep for the given duration then behave as the wrapped scenario.
    Delay {
        duration: Duration,
        inner: Box<Scenario>,
    },
}

impl Scenario {
    /// Shortcut: a successful scenario with default usage metrics and
    /// default delivery config.
    pub fn ok(content: impl Into<String>, model: impl Into<String>) -> Self {
        Self::Ok {
            content_blocks: vec![RawContentBlock::Text(content.into())],
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: 10,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: DeliveryConfig::default(),
            include_usage: false,
            protocol: ProtocolId::new("openai"),
            segment_granularity: 0,
        }
    }

    /// Shortcut: an error scenario with default zero usage.
    pub fn err(error: ProviderError) -> Self {
        Self::Err {
            error,
            prompt_tokens: 0,
            completion_tokens: 0,
        }
    }

    /// Error scenario with custom usage metrics.
    pub fn err_with(error: ProviderError, prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self::Err {
            error,
            prompt_tokens,
            completion_tokens,
        }
    }

    /// Shortcut: a delayed scenario — sleeps for `duration` then resolves
    /// as `inner`.
    pub fn delay(duration: Duration, inner: Scenario) -> Self {
        Self::Delay {
            duration,
            inner: Box::new(inner),
        }
    }

    /// Returns usage as [`RawUsage`] (for the new Provider trait).
    pub(crate) fn raw_usage(&self) -> RawUsage {
        match self {
            Self::Ok {
                prompt_tokens,
                completion_tokens,
                cache_read_tokens,
                cache_write_tokens,
                ..
            } => RawUsage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                total_tokens: Some(*prompt_tokens + *completion_tokens),
                cache_read_tokens: *cache_read_tokens,
                cache_write_tokens: *cache_write_tokens,
            },
            Self::Err {
                prompt_tokens,
                completion_tokens,
                ..
            } => RawUsage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                total_tokens: Some(*prompt_tokens + *completion_tokens),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            Self::Delay { inner, .. } => inner.raw_usage(),
        }
    }
}

impl Clone for Scenario {
    fn clone(&self) -> Self {
        match self {
            Self::Ok {
                content_blocks,
                model,
                prompt_tokens,
                completion_tokens,
                cache_read_tokens,
                cache_write_tokens,
                delivery,
                include_usage,
                protocol,
                segment_granularity,
            } => Self::Ok {
                content_blocks: content_blocks.clone(),
                model: model.clone(),
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                cache_read_tokens: *cache_read_tokens,
                cache_write_tokens: *cache_write_tokens,
                delivery: delivery.clone(),
                include_usage: *include_usage,
                protocol: protocol.clone(),
                segment_granularity: *segment_granularity,
            },
            Self::Err {
                error,
                prompt_tokens,
                completion_tokens,
            } => {
                // ProviderError doesn't impl Clone; reconstruct from Display
                let err_str = format!("{}", error);
                Self::Err {
                    error: ProviderError::Legacy(err_str),
                    prompt_tokens: *prompt_tokens,
                    completion_tokens: *completion_tokens,
                }
            }
            Self::Delay { duration, inner } => Self::Delay {
                duration: *duration,
                inner: inner.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delivery_config_default() {
        let config = DeliveryConfig::default();
        assert!(config.first_token_delay.is_none());
        assert!(config.per_segment_delay.is_none());
        assert!(config.overall_delay.is_none());
        assert!(config.error_injection.is_none());
        assert!(config.stream_interrupt.is_none());
    }

    #[test]
    fn test_delivery_config_clone() {
        let config = DeliveryConfig {
            first_token_delay: Some(Duration::from_millis(100)),
            per_segment_delay: Some(Duration::from_millis(50)),
            overall_delay: None,
            error_injection: Some(ErrorInjection {
                status_code: 429,
                message: "rate limited".into(),
                retry_after: Some(30),
            }),
            stream_interrupt: Some(StreamInterrupt {
                interrupt_after_frames: 3,
            }),
        };
        let cloned = config.clone();
        assert_eq!(cloned.first_token_delay, Some(Duration::from_millis(100)));
        assert_eq!(cloned.per_segment_delay, Some(Duration::from_millis(50)));
        assert!(cloned.overall_delay.is_none());
        let ei = cloned.error_injection.unwrap();
        assert_eq!(ei.status_code, 429);
        assert_eq!(ei.message, "rate limited");
        assert_eq!(ei.retry_after, Some(30));
        let si = cloned.stream_interrupt.unwrap();
        assert_eq!(si.interrupt_after_frames, 3);
    }

    #[test]
    fn test_scenario_ok_defaults() {
        let scenario = Scenario::ok("hello", "model-x");
        match scenario {
            Scenario::Ok {
                content_blocks,
                model,
                prompt_tokens,
                completion_tokens,
                cache_read_tokens,
                cache_write_tokens,
                delivery,
                include_usage,
                protocol,
                segment_granularity,
            } => {
                assert_eq!(content_blocks, vec![RawContentBlock::Text("hello".into())]);
                assert_eq!(model, "model-x");
                assert_eq!(prompt_tokens, 10);
                assert_eq!(completion_tokens, 10);
                assert!(cache_read_tokens.is_none());
                assert!(cache_write_tokens.is_none());
                assert!(delivery.first_token_delay.is_none());
                assert!(delivery.per_segment_delay.is_none());
                assert!(delivery.overall_delay.is_none());
                assert!(delivery.error_injection.is_none());
                assert!(delivery.stream_interrupt.is_none());
                assert!(!include_usage);
                assert_eq!(segment_granularity, 0);
                // Default protocol should be openai
                assert_eq!(format!("{}", protocol), "openai");
            }
            _ => panic!("Expected Scenario::Ok"),
        }
    }

    /// Create a fully-configured Scenario::Ok for testing clone/usage.
    fn make_full_scenario() -> Scenario {
        Scenario::Ok {
            content_blocks: vec![RawContentBlock::Text("test content".into())],
            model: "test-model".into(),
            prompt_tokens: 20,
            completion_tokens: 30,
            cache_read_tokens: Some(10),
            cache_write_tokens: Some(5),
            delivery: DeliveryConfig {
                first_token_delay: Some(Duration::from_millis(200)),
                per_segment_delay: Some(Duration::from_millis(100)),
                overall_delay: None,
                error_injection: Some(ErrorInjection {
                    status_code: 401,
                    message: "unauthorized".into(),
                    retry_after: None,
                }),
                stream_interrupt: None,
            },
            include_usage: true,
            protocol: ProtocolId::new("anthropic"),
            segment_granularity: 5,
        }
    }

    /// Clone a Scenario::Ok and assert all fields are preserved.
    fn assert_scenario_ok_clone(original: Scenario) {
        let cloned = original.clone();
        match cloned {
            Scenario::Ok {
                content_blocks,
                model,
                prompt_tokens,
                completion_tokens,
                cache_read_tokens,
                cache_write_tokens,
                delivery,
                include_usage,
                protocol,
                segment_granularity,
            } => {
                assert_eq!(
                    content_blocks,
                    vec![RawContentBlock::Text("test content".into())]
                );
                assert_eq!(model, "test-model");
                assert_eq!(prompt_tokens, 20);
                assert_eq!(completion_tokens, 30);
                assert_eq!(cache_read_tokens, Some(10));
                assert_eq!(cache_write_tokens, Some(5));
                assert_eq!(delivery.first_token_delay, Some(Duration::from_millis(200)));
                assert_eq!(delivery.per_segment_delay, Some(Duration::from_millis(100)));
                assert!(delivery.overall_delay.is_none());
                let ei = delivery.error_injection.unwrap();
                assert_eq!(ei.status_code, 401);
                assert!(delivery.stream_interrupt.is_none());
                assert!(include_usage);
                assert_eq!(format!("{}", protocol), "anthropic");
                assert_eq!(segment_granularity, 5);
            }
            _ => panic!("Expected Scenario::Ok"),
        }
    }

    #[test]
    fn test_scenario_ok_clone() {
        assert_scenario_ok_clone(make_full_scenario());
    }

    #[test]
    fn test_scenario_err_clone() {
        let scenario = Scenario::err(ProviderError::Legacy("test".into()));
        let cloned = scenario.clone();
        match cloned {
            Scenario::Err {
                error,
                prompt_tokens,
                completion_tokens,
            } => {
                assert!(format!("{}", error).contains("test"));
                assert_eq!(prompt_tokens, 0);
                assert_eq!(completion_tokens, 0);
            }
            _ => panic!("Expected Scenario::Err"),
        }
    }

    #[test]
    fn test_scenario_delay_clone() {
        let inner = Scenario::ok("inner", "m");
        let scenario = Scenario::delay(Duration::from_millis(50), inner);
        let cloned = scenario.clone();
        match cloned {
            Scenario::Delay { duration, inner } => {
                assert_eq!(duration, Duration::from_millis(50));
                // Verify inner is an Ok scenario by checking raw_usage
                let usage = inner.raw_usage();
                assert_eq!(usage.prompt_tokens, 10);
            }
            _ => panic!("Expected Scenario::Delay"),
        }
    }

    #[test]
    fn test_raw_usage_ok() {
        let scenario = Scenario::ok("x", "m");
        let usage = scenario.raw_usage();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 10);
        assert_eq!(usage.total_tokens, Some(20));
    }

    #[test]
    fn test_raw_usage_err() {
        let scenario = Scenario::err_with(ProviderError::Legacy("e".into()), 5, 3);
        let usage = scenario.raw_usage();
        assert_eq!(usage.prompt_tokens, 5);
        assert_eq!(usage.completion_tokens, 3);
        assert_eq!(usage.total_tokens, Some(8));
    }

    #[test]
    fn test_raw_usage_delay() {
        let inner = Scenario::Ok {
            content_blocks: vec![RawContentBlock::Text("c".into())],
            model: "m".into(),
            prompt_tokens: 7,
            completion_tokens: 3,
            cache_read_tokens: None,
            cache_write_tokens: None,
            delivery: DeliveryConfig::default(),
            include_usage: false,
            protocol: ProtocolId::new("openai"),
            segment_granularity: 0,
        };
        let scenario = Scenario::delay(Duration::from_millis(10), inner);
        let usage = scenario.raw_usage();
        assert_eq!(usage.prompt_tokens, 7);
        assert_eq!(usage.completion_tokens, 3);
        assert_eq!(usage.total_tokens, Some(10));
    }
}
