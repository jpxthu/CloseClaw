//! OpenAI `/v1/chat/completions` endpoint handler.
//!
//! Parses the OpenAI chat completion request body via the protocol module,
//! extracts protocol-agnostic `RequestFeatures`, delegates to the scenario
//! engine, and returns the appropriate response via the delivery layer.

use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::Sse;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, Json};

use crate::delivery::{self, DeliveryConfig, DeliveryResult, Protocol, StreamInterrupt};
use crate::protocol::openai::{extract_request_features, ChatCompletionRequest};
use crate::scenario::{DecisionOutcome, ScenarioState};

use delivery::{SseEventStream, DEFAULT_SEGMENT_GRANULARITY};

/// Handler for POST `/v1/chat/completions`.
///
/// Extracts request features, delegates to the scenario engine via shared state,
/// and routes through the delivery layer for streaming, non-streaming, or error
/// responses (with optional Retry-After header).
pub async fn handler(
    State(state): State<ScenarioState>,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Response, (StatusCode, HeaderMap, String)> {
    let features = extract_request_features(&req);

    let outcome = {
        let mut engine = state.engine.lock().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                HeaderMap::new(),
                e.to_string(),
            )
        })?;
        engine.decide(&features)
    };

    match outcome {
        DecisionOutcome::Error(e) => {
            let status =
                StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            Err((status, HeaderMap::new(), e.message))
        }
        DecisionOutcome::Decision(decision) => {
            let stream_interrupt = decision
                .stream_interrupt_after
                .map(|n| StreamInterrupt { after_event: n });
            let include_usage = req
                .stream_options
                .as_ref()
                .is_some_and(|opts| opts.include_usage);
            let config = DeliveryConfig {
                segment_granularity: decision
                    .segment_granularity
                    .unwrap_or(DEFAULT_SEGMENT_GRANULARITY),
                include_usage,
                stream_interrupt,
            };

            let result = delivery::deliver(&decision, Protocol::OpenAi, &config).await;

            match result {
                DeliveryResult::SseStreamWithConfig {
                    events,
                    segment_delay_ms,
                    max_events,
                } => {
                    let stream = SseEventStream::new(events)
                        .with_segment_delay(segment_delay_ms)
                        .with_max_events(max_events);
                    Ok(Sse::new(stream).into_response())
                }
                DeliveryResult::SseStream(events) => {
                    let stream = SseEventStream::new(events);
                    Ok(Sse::new(stream).into_response())
                }
                DeliveryResult::JsonResponse(json) => Ok(Json(json).into_response()),
                DeliveryResult::HttpError {
                    status,
                    message,
                    retry_after,
                } => {
                    let code =
                        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                    let mut headers = HeaderMap::new();
                    if let Some(secs) = retry_after {
                        if let Ok(val) = secs.to_string().parse() {
                            headers.insert(axum::http::header::RETRY_AFTER, val);
                        }
                    }
                    Err((code, headers, message))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::openai::{
        build_chat_completion_response, extract_request_features, ChatCompletionRequest,
        StreamOptions,
    };

    #[test]
    fn handler_delegates_to_protocol() {
        // Verify the endpoint handler compiles and can invoke protocol functions.
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            stream: false,
            max_tokens: Some(1024),
            temperature: Some(0.7),
            tools: None,
            stop: None,
            stream_options: None,
        };
        let features = extract_request_features(&req);
        assert_eq!(features.model, "gpt-4");

        // Verify response serializes to valid JSON (fields are private, test via serde)
        let resp = build_chat_completion_response(&req.model);
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert!(json["choices"].is_array());
        assert_eq!(json["choices"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn stream_options_none_yields_include_usage_false() {
        // When stream_options is absent, include_usage should default to false
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            stream: true,
            max_tokens: None,
            temperature: None,
            tools: None,
            stop: None,
            stream_options: None,
        };
        let include_usage = req
            .stream_options
            .as_ref()
            .map_or(false, |opts| opts.include_usage);
        assert!(!include_usage);
    }

    #[test]
    fn stream_options_include_usage_true() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            stream: true,
            max_tokens: None,
            temperature: None,
            tools: None,
            stop: None,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
        };
        let include_usage = req
            .stream_options
            .as_ref()
            .map_or(false, |opts| opts.include_usage);
        assert!(include_usage);
    }

    #[test]
    fn stream_options_include_usage_false() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            stream: true,
            max_tokens: None,
            temperature: None,
            tools: None,
            stop: None,
            stream_options: Some(StreamOptions {
                include_usage: false,
            }),
        };
        let include_usage = req
            .stream_options
            .as_ref()
            .map_or(false, |opts| opts.include_usage);
        assert!(!include_usage);
    }
}
