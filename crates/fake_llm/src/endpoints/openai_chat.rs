//! OpenAI `/v1/chat/completions` endpoint handler.
//!
//! Parses the OpenAI chat completion request body via the protocol module,
//! extracts protocol-agnostic `RequestFeatures`, delegates to the scenario
//! engine, and returns the appropriate response via the delivery layer.

use std::convert::Infallible;

use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::{extract::State, Json};

use crate::delivery::{self, DeliveryConfig, DeliveryResult, Protocol};
use crate::protocol::openai::{extract_request_features, ChatCompletionRequest};
use crate::scenario::{DecisionOutcome, ScenarioState};

/// Default segment granularity for streaming content splitting.
const DEFAULT_SEGMENT_GRANULARITY: usize = 20;

/// Wrapper that yields SSE events from a `Vec`, implementing `futures::Stream`.
struct SseEventStream {
    inner: std::vec::IntoIter<delivery::SseEvent>,
}

impl futures_core::Stream for SseEventStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.inner.next() {
            Some(e) => std::task::Poll::Ready(Some(Ok(to_axum_event(e)))),
            None => std::task::Poll::Ready(None),
        }
    }
}

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
            let config = DeliveryConfig {
                segment_granularity: DEFAULT_SEGMENT_GRANULARITY,
                include_usage: true,
            };

            let result = delivery::deliver(&decision, Protocol::OpenAi, &config).await;

            match result {
                DeliveryResult::SseStream(events) => {
                    let stream = SseEventStream {
                        inner: events.into_iter(),
                    };
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

/// Convert a delivery `SseEvent` into an axum `Event`.
///
/// Maps the event type and data fields into the SSE wire format.
fn to_axum_event(e: delivery::SseEvent) -> Event {
    let mut event = Event::default();
    if !e.event_type.is_empty() {
        event = event.event(e.event_type);
    }
    event.data(e.data)
}

#[cfg(test)]
mod tests {
    use crate::protocol::openai::{
        build_chat_completion_response, extract_request_features, ChatCompletionRequest,
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
}
