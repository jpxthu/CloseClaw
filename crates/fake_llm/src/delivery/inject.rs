//! Delivery layer — delay injection, error injection, and unified delivery.
//!
//! Provides `apply_delay()`, `DeliveryResult`, and the unified
//! `deliver()` entry point that routes through delay injection,
//! error injection, and response generation.

use axum::http::StatusCode;

use super::sse::{generate_anthropic_sse, generate_openai_sse, SseEvent};

// ---------------------------------------------------------------------------
// Delay injection
// ---------------------------------------------------------------------------

/// Execute an optional artificial delay.
///
/// When `ms` is `Some(n)`, sleeps for `n` milliseconds before returning.
/// When `ms` is `None` or `Some(0)`, returns immediately.
pub async fn apply_delay(ms: Option<u64>) {
    if let Some(n) = ms {
        if n > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(n)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Delivery result
// ---------------------------------------------------------------------------

/// Result of the delivery layer's decision.
///
/// The endpoint handler pattern-matches on this to build the appropriate
/// Axum response type (SSE stream, JSON body, or HTTP error).
pub enum DeliveryResult {
    /// Streaming response: SSE events ready to be sent over the channel.
    SseStream(Vec<SseEvent>),
    /// Streaming response with delivery config (segment delay, interrupt).
    SseStreamWithConfig {
        events: Vec<SseEvent>,
        segment_delay_ms: u64,
        max_events: Option<usize>,
    },
    /// Non-streaming JSON response body.
    JsonResponse(serde_json::Value),
    /// HTTP error with status code, message body, and optional Retry-After header.
    HttpError {
        status: u16,
        message: String,
        retry_after: Option<u64>,
    },
}

impl DeliveryResult {
    /// Build an Axum-compatible error response tuple.
    ///
    /// Returns `(StatusCode, HeaderMap, String)` suitable for Axum's
    /// `Result<Json<T>, (StatusCode, HeaderMap, String)>` handler return type.
    pub fn into_axum_error(self) -> (StatusCode, axum::http::HeaderMap, String) {
        match self {
            DeliveryResult::HttpError {
                status,
                message,
                retry_after,
            } => {
                let code =
                    StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let mut headers = axum::http::HeaderMap::new();
                if let Some(secs) = retry_after {
                    headers.insert(
                        axum::http::header::RETRY_AFTER,
                        secs.to_string().parse().unwrap(),
                    );
                }
                (code, headers, message)
            }
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::http::HeaderMap::new(),
                "unexpected non-error delivery result".to_string(),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Models endpoint delivery
// ---------------------------------------------------------------------------

/// Decision from the models endpoint scenario engine, consumed by the
/// delivery layer for unified delay and error injection.
pub struct ModelsDeliveryDecision {
    /// Scenario-declared model entries (None = placeholder path).
    pub models: Option<Vec<crate::scenario::types::ModelEntry>>,
    /// Optional HTTP error injection.
    pub http_error: Option<crate::scenario::types::HttpError>,
    /// Optional overall delay before responding.
    pub delay: Option<u64>,
}

/// Models-list JSON response body.
fn build_models_json(entries: &[crate::scenario::types::ModelEntry]) -> serde_json::Value {
    use crate::protocol::openai::{ModelObject, ModelsResponse};

    let resp = ModelsResponse {
        object: "list".to_string(),
        data: entries
            .iter()
            .map(|e| ModelObject {
                id: e.id.clone(),
                object: "model".to_string(),
                created: 0,
                owned_by: e.owned_by.clone(),
            })
            .collect(),
    };
    serde_json::to_value(&resp).unwrap_or_default()
}

/// Delivery entry point for the `/v1/models` endpoint.
///
/// Mirrors the `deliver()` flow — delay injection, error injection,
/// then JSON response — but produces a models-list JSON body instead
/// of a chat completion body.
///
/// When `models` is `Some`, returns the model list JSON. When `None`,
/// returns the default placeholder models list.
pub async fn deliver_models(decision: &ModelsDeliveryDecision) -> DeliveryResult {
    // 1. Execute delay injection
    apply_delay(decision.delay).await;

    // 2. Check error injection
    if let Some(ref err) = decision.http_error {
        return DeliveryResult::HttpError {
            status: err.status,
            message: err.message.clone(),
            retry_after: err.retry_after,
        };
    }

    // 3. Build models response
    match &decision.models {
        Some(entries) => DeliveryResult::JsonResponse(build_models_json(entries)),
        None => {
            use crate::protocol::openai::build_models_response;
            DeliveryResult::JsonResponse(
                serde_json::to_value(build_models_response()).unwrap_or_default(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Unified delivery entry point
// ---------------------------------------------------------------------------

/// Protocol identifier for SSE generation.
pub enum Protocol {
    OpenAi,
    Anthropic,
}

/// Stream interrupt configuration.
///
/// Declares at which position the streaming response should be truncated.
pub struct StreamInterrupt {
    /// Number of events to emit before disconnecting (0 = first event then stop).
    pub after_event: usize,
}

/// Configuration for the delivery layer.
pub struct DeliveryConfig {
    /// Segment granularity for content splitting in streaming mode.
    pub segment_granularity: usize,
    /// Whether to include usage in the final streaming chunk.
    pub include_usage: bool,
    /// Optional stream interrupt: truncate the SSE stream after N events.
    pub stream_interrupt: Option<StreamInterrupt>,
}

/// Unified delivery entry point.
///
/// 1. Executes delay injection (if configured in the decision).
/// 2. Checks for HTTP error injection and returns `HttpError` immediately.
/// 3. For streaming requests: generates SSE events via the protocol layer.
/// 4. For non-streaming requests: returns the JSON response body.
pub async fn deliver(
    decision: &crate::types::ScenarioDecision,
    protocol: Protocol,
    config: &DeliveryConfig,
) -> DeliveryResult {
    // 1. Execute delay injection
    apply_delay(decision.delay).await;

    // 2. Check error injection
    if let Some(ref err) = decision.http_error {
        return DeliveryResult::HttpError {
            status: err.status,
            message: err.message.clone(),
            retry_after: err.retry_after,
        };
    }

    // 3. Build response
    let usage = decision.usage.clone().unwrap_or_default();

    if decision.stream {
        // Streaming path:
        // 1. First token delay (before generating events)
        apply_delay(decision.first_token_delay).await;

        // 2. Generate SSE events
        let events = match protocol {
            Protocol::OpenAi => generate_openai_sse(
                &decision.response_blocks,
                &decision.model,
                &usage,
                config.include_usage,
                config.segment_granularity,
            ),
            Protocol::Anthropic => generate_anthropic_sse(
                &decision.response_blocks,
                &decision.model,
                &usage,
                config.segment_granularity,
            ),
        };

        // 3. Build stream with segment delay and optional interrupt
        let max_events = config.stream_interrupt.as_ref().map(|si| si.after_event);
        let segment_delay = decision.segment_delay.unwrap_or(0);
        DeliveryResult::SseStreamWithConfig {
            events,
            segment_delay_ms: segment_delay,
            max_events,
        }
    } else {
        // Non-streaming: build JSON response via protocol layer
        let json = match protocol {
            Protocol::OpenAi => {
                let resp =
                    crate::protocol::openai::build_chat_completion_response_from_decision(decision);
                serde_json::to_value(&resp).unwrap_or_default()
            }
            Protocol::Anthropic => {
                let resp =
                    crate::protocol::anthropic::build_message_response_from_decision(decision);
                serde_json::to_value(&resp).unwrap_or_default()
            }
        };
        DeliveryResult::JsonResponse(json)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::types::HttpError;
    use crate::scenario::types::ResponseBlock;

    fn text_block(content: &str) -> ResponseBlock {
        ResponseBlock {
            block_type: "text".to_string(),
            content: Some(content.to_string()),
            tool_name: None,
            tool_arguments: None,
            reasoning: None,
            signature: None,
        }
    }

    fn default_usage() -> crate::scenario::types::UsageResponse {
        crate::scenario::types::UsageResponse {
            prompt_tokens: Some(10),
            completion_tokens: Some(20),
            reasoning_tokens: None,
            cache_hit_tokens: None,
            cache_write_tokens: None,
            cache_fields_missing: false,
        }
    }

    // ------------------------------------------------------------------
    // apply_delay
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn apply_delay_none_is_immediate() {
        let start = std::time::Instant::now();
        apply_delay(None).await;
        assert!(start.elapsed().as_millis() < 50);
    }

    #[tokio::test]
    async fn apply_delay_zero_is_immediate() {
        let start = std::time::Instant::now();
        apply_delay(Some(0)).await;
        assert!(start.elapsed().as_millis() < 50);
    }

    #[tokio::test]
    async fn apply_delay_executes_sleep() {
        let start = std::time::Instant::now();
        apply_delay(Some(100)).await;
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 80, "expected >= 80ms, got {}ms", elapsed);
    }

    // ------------------------------------------------------------------
    // DeliveryResult
    // ------------------------------------------------------------------

    #[test]
    fn delivery_result_http_error_into_axum_error() {
        let result = DeliveryResult::HttpError {
            status: 429,
            message: "rate limited".to_string(),
            retry_after: Some(60),
        };
        let (code, headers, msg) = result.into_axum_error();
        assert_eq!(code, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(msg, "rate limited");
        let retry = headers
            .get(axum::http::header::RETRY_AFTER)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(retry, "60");
    }

    #[test]
    fn delivery_result_http_error_no_retry_after() {
        let result = DeliveryResult::HttpError {
            status: 500,
            message: "server error".to_string(),
            retry_after: None,
        };
        let (code, headers, msg) = result.into_axum_error();
        assert_eq!(code, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(msg, "server error");
        assert!(headers.get(axum::http::header::RETRY_AFTER).is_none());
    }

    // ------------------------------------------------------------------
    // deliver — streaming path
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_streaming_openai() {
        let decision = crate::types::ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "test".to_string(),
            stream: true,
            response_blocks: vec![text_block("Hello!")],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: Some(default_usage()),
        };
        let config = DeliveryConfig {
            segment_granularity: 0,
            include_usage: false,
            stream_interrupt: None,
        };
        let result = deliver(&decision, Protocol::OpenAi, &config).await;
        match result {
            DeliveryResult::SseStreamWithConfig { events, .. } => {
                // role chunk, content delta, finish, [DONE]
                assert_eq!(events.len(), 4);
                assert_eq!(events[3].data, "[DONE]");
            }
            _ => panic!("expected SseStreamWithConfig"),
        }
    }

    #[tokio::test]
    async fn deliver_streaming_anthropic() {
        let decision = crate::types::ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "test".to_string(),
            stream: true,
            response_blocks: vec![text_block("Hi!")],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: Some(default_usage()),
        };
        let config = DeliveryConfig {
            segment_granularity: 0,
            include_usage: false,
            stream_interrupt: None,
        };
        let result = deliver(&decision, Protocol::Anthropic, &config).await;
        match result {
            DeliveryResult::SseStreamWithConfig { events, .. } => {
                // message_start, content_block_start, ping, text_delta,
                // content_block_stop, message_delta, message_stop
                assert_eq!(events.len(), 7);
            }
            _ => panic!("expected SseStreamWithConfig"),
        }
    }

    // ------------------------------------------------------------------
    // deliver — non-streaming path
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_non_streaming_openai() {
        let decision = crate::types::ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "test".to_string(),
            stream: false,
            response_blocks: vec![text_block("Hello!")],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };
        let config = DeliveryConfig {
            segment_granularity: 0,
            include_usage: false,
            stream_interrupt: None,
        };
        let result = deliver(&decision, Protocol::OpenAi, &config).await;
        match result {
            DeliveryResult::JsonResponse(json) => {
                assert_eq!(json["object"], "chat.completion");
                assert_eq!(json["choices"][0]["message"]["content"], "Hello!");
            }
            _ => panic!("expected JsonResponse"),
        }
    }

    #[tokio::test]
    async fn deliver_non_streaming_anthropic() {
        let decision = crate::types::ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "test".to_string(),
            stream: false,
            response_blocks: vec![text_block("Hi!")],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };
        let config = DeliveryConfig {
            segment_granularity: 0,
            include_usage: false,
            stream_interrupt: None,
        };
        let result = deliver(&decision, Protocol::Anthropic, &config).await;
        match result {
            DeliveryResult::JsonResponse(json) => {
                assert_eq!(json["type"], "message");
                assert_eq!(json["role"], "assistant");
            }
            _ => panic!("expected JsonResponse"),
        }
    }

    // ------------------------------------------------------------------
    // deliver — error injection path
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_error_injection() {
        let decision = crate::types::ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "test".to_string(),
            stream: false,
            response_blocks: vec![],
            http_error: Some(HttpError {
                status: 429,
                message: "rate limited".to_string(),
                retry_after: None,
            }),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };
        let config = DeliveryConfig {
            segment_granularity: 0,
            include_usage: false,
            stream_interrupt: None,
        };
        let result = deliver(&decision, Protocol::OpenAi, &config).await;
        match result {
            DeliveryResult::HttpError {
                status, message, ..
            } => {
                assert_eq!(status, 429);
                assert_eq!(message, "rate limited");
            }
            _ => panic!("expected HttpError"),
        }
    }

    // ------------------------------------------------------------------
    // deliver — delay injection path
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_with_delay() {
        let decision = crate::types::ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "test".to_string(),
            stream: false,
            response_blocks: vec![text_block("ok")],
            http_error: None,
            delay: Some(100),
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };
        let config = DeliveryConfig {
            segment_granularity: 0,
            include_usage: false,
            stream_interrupt: None,
        };
        let start = std::time::Instant::now();
        let result = deliver(&decision, Protocol::OpenAi, &config).await;
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 80, "expected >= 80ms, got {}ms", elapsed);
        match result {
            DeliveryResult::JsonResponse(_) => {}
            _ => panic!("expected JsonResponse"),
        }
    }

    // ------------------------------------------------------------------
    // deliver — retry_after propagation
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_error_with_retry_after() {
        let decision = crate::types::ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "test".to_string(),
            stream: false,
            response_blocks: vec![],
            http_error: Some(HttpError {
                status: 429,
                message: "rate limited".to_string(),
                retry_after: Some(60),
            }),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };
        let config = DeliveryConfig {
            segment_granularity: 0,
            include_usage: false,
            stream_interrupt: None,
        };
        let result = deliver(&decision, Protocol::OpenAi, &config).await;
        match result {
            DeliveryResult::HttpError {
                status,
                message,
                retry_after,
            } => {
                assert_eq!(status, 429);
                assert_eq!(message, "rate limited");
                assert_eq!(retry_after, Some(60));
            }
            _ => panic!("expected HttpError"),
        }
    }

    #[tokio::test]
    async fn deliver_error_without_retry_after() {
        let decision = crate::types::ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "test".to_string(),
            stream: false,
            response_blocks: vec![],
            http_error: Some(HttpError {
                status: 500,
                message: "error".to_string(),
                retry_after: None,
            }),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };
        let config = DeliveryConfig {
            segment_granularity: 0,
            include_usage: false,
            stream_interrupt: None,
        };
        let result = deliver(&decision, Protocol::OpenAi, &config).await;
        match result {
            DeliveryResult::HttpError { retry_after, .. } => {
                assert!(retry_after.is_none());
            }
            _ => panic!("expected HttpError"),
        }
    }

    // ------------------------------------------------------------------
    // deliver_models — placeholder path
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_models_placeholder() {
        let decision = ModelsDeliveryDecision {
            models: None,
            http_error: None,
            delay: None,
        };
        let result = deliver_models(&decision).await;
        match result {
            DeliveryResult::JsonResponse(json) => {
                assert_eq!(json["object"], "list");
                assert!(json["data"].as_array().unwrap().len() > 0);
            }
            _ => panic!("expected JsonResponse"),
        }
    }

    // ------------------------------------------------------------------
    // deliver_models — scenario-declared models
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_models_with_entries() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![
                crate::scenario::types::ModelEntry {
                    id: "gpt-4".to_string(),
                    owned_by: "openai".to_string(),
                },
                crate::scenario::types::ModelEntry {
                    id: "claude-3".to_string(),
                    owned_by: "anthropic".to_string(),
                },
            ]),
            http_error: None,
            delay: None,
        };
        let result = deliver_models(&decision).await;
        match result {
            DeliveryResult::JsonResponse(json) => {
                assert_eq!(json["object"], "list");
                let data = json["data"].as_array().unwrap();
                assert_eq!(data.len(), 2);
                assert_eq!(data[0]["id"], "gpt-4");
                assert_eq!(data[1]["id"], "claude-3");
            }
            _ => panic!("expected JsonResponse"),
        }
    }

    // ------------------------------------------------------------------
    // deliver_models — error injection
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_models_auth_failure() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![]),
            http_error: Some(HttpError {
                status: 401,
                message: "unauthorized".to_string(),
                retry_after: None,
            }),
            delay: None,
        };
        let result = deliver_models(&decision).await;
        match result {
            DeliveryResult::HttpError {
                status,
                message,
                retry_after,
            } => {
                assert_eq!(status, 401);
                assert_eq!(message, "unauthorized");
                assert!(retry_after.is_none());
            }
            _ => panic!("expected HttpError"),
        }
    }

    #[tokio::test]
    async fn deliver_models_rate_limited_with_retry_after() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![]),
            http_error: Some(HttpError {
                status: 429,
                message: "rate limited".to_string(),
                retry_after: Some(60),
            }),
            delay: None,
        };
        let result = deliver_models(&decision).await;
        match result {
            DeliveryResult::HttpError {
                status,
                message,
                retry_after,
            } => {
                assert_eq!(status, 429);
                assert_eq!(message, "rate limited");
                assert_eq!(retry_after, Some(60));
            }
            _ => panic!("expected HttpError"),
        }
    }

    #[tokio::test]
    async fn deliver_models_server_error() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![]),
            http_error: Some(HttpError {
                status: 500,
                message: "internal error".to_string(),
                retry_after: None,
            }),
            delay: None,
        };
        let result = deliver_models(&decision).await;
        match result {
            DeliveryResult::HttpError {
                status, message, ..
            } => {
                assert_eq!(status, 500);
                assert_eq!(message, "internal error");
            }
            _ => panic!("expected HttpError"),
        }
    }

    // ------------------------------------------------------------------
    // deliver_models — delay injection
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn deliver_models_with_delay() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![crate::scenario::types::ModelEntry {
                id: "gpt-4".to_string(),
                owned_by: "openai".to_string(),
            }]),
            http_error: None,
            delay: Some(100),
        };
        let start = std::time::Instant::now();
        let result = deliver_models(&decision).await;
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 80, "expected >= 80ms, got {}ms", elapsed);
        match result {
            DeliveryResult::JsonResponse(json) => {
                assert_eq!(json["data"][0]["id"], "gpt-4");
            }
            _ => panic!("expected JsonResponse"),
        }
    }

    #[tokio::test]
    async fn deliver_models_delay_then_error() {
        // Error injection overrides models: delay is applied first,
        // then error is returned.
        let decision = ModelsDeliveryDecision {
            models: Some(vec![]),
            http_error: Some(HttpError {
                status: 401,
                message: "unauthorized".to_string(),
                retry_after: None,
            }),
            delay: Some(100),
        };
        let start = std::time::Instant::now();
        let result = deliver_models(&decision).await;
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 80, "expected >= 80ms, got {}ms", elapsed);
        match result {
            DeliveryResult::HttpError { status, .. } => {
                assert_eq!(status, 401);
            }
            _ => panic!("expected HttpError"),
        }
    }

    #[tokio::test]
    async fn deliver_models_no_delay_no_error() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![crate::scenario::types::ModelEntry {
                id: "m1".to_string(),
                owned_by: "org".to_string(),
            }]),
            http_error: None,
            delay: None,
        };
        let start = std::time::Instant::now();
        let result = deliver_models(&decision).await;
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed < 50, "expected < 50ms, got {}ms", elapsed);
        match result {
            DeliveryResult::JsonResponse(json) => {
                assert_eq!(json["data"][0]["id"], "m1");
            }
            _ => panic!("expected JsonResponse"),
        }
    }
}
