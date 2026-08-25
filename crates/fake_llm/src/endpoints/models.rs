//! `/v1/models` endpoint handler.
//!
//! Returns a model list in OpenAI format. When a scenario declares a
//! `models` field, that list is returned instead of the default placeholder.
//! Error injection and delay injection are handled by the delivery layer.

use axum::response::{IntoResponse, Response};
use axum::{extract::State, Json};

use crate::delivery::{self, DeliveryResult, ModelsDeliveryDecision};
use crate::scenario::{ModelsDecision, ScenarioState};

use super::endpoint_error::EndpointError;

/// Handler for GET `/v1/models`.
///
/// Delegates to the scenario engine for deterministic model lists and
/// routes through the delivery layer for error injection and delay injection.
pub async fn handler(State(state): State<ScenarioState>) -> Result<Response, EndpointError> {
    let outcome = {
        let mut engine = state
            .engine
            .lock()
            .map_err(|e| EndpointError::internal(e.to_string()))?;
        engine.decide_for_models()
    };

    // Convert engine decision to delivery-layer decision.
    let delivery_decision = match outcome {
        ModelsDecision::Placeholder => ModelsDeliveryDecision {
            models: None,
            http_error: None,
            delay: None,
        },
        ModelsDecision::Error(e) => ModelsDeliveryDecision {
            models: None,
            http_error: Some(e),
            delay: None,
        },
        ModelsDecision::Models(entries, delay) => ModelsDeliveryDecision {
            models: Some(entries),
            http_error: None,
            delay,
        },
    };

    // Route through the unified delivery layer.
    let result = delivery::deliver_models(&delivery_decision).await;

    match result {
        DeliveryResult::JsonResponse(json) => Ok(Json(json).into_response()),
        DeliveryResult::HttpError {
            status,
            message,
            retry_after,
        } => Err(EndpointError::http(status, retry_after, message)),
        _ => Err(EndpointError::internal(
            "unexpected delivery result for models endpoint",
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::delivery::{DeliveryResult, ModelsDeliveryDecision};
    use crate::scenario::types::{HttpError, ModelEntry};

    #[tokio::test]
    async fn handler_placeholder_returns_models_list() {
        let decision = ModelsDeliveryDecision {
            models: None,
            http_error: None,
            delay: None,
        };
        let result = crate::delivery::deliver_models(&decision).await;
        match result {
            DeliveryResult::JsonResponse(json) => {
                assert_eq!(json["object"], "list");
                // Default placeholder has at least one model
                assert!(json["data"].as_array().unwrap().len() > 0);
            }
            _ => panic!("expected JsonResponse for placeholder"),
        }
    }

    #[tokio::test]
    async fn handler_auth_failure_returns_401() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![]),
            http_error: Some(HttpError {
                status: 401,
                message: "unauthorized".to_string(),
                retry_after: None,
            }),
            delay: None,
        };
        let result = crate::delivery::deliver_models(&decision).await;
        match result {
            DeliveryResult::HttpError { status, .. } => {
                assert_eq!(status, 401);
            }
            _ => panic!("expected HttpError for auth failure"),
        }
    }

    #[tokio::test]
    async fn handler_rate_limited_returns_429_with_retry_after() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![]),
            http_error: Some(HttpError {
                status: 429,
                message: "rate limited".to_string(),
                retry_after: Some(60),
            }),
            delay: None,
        };
        let result = crate::delivery::deliver_models(&decision).await;
        match result {
            DeliveryResult::HttpError {
                status,
                retry_after,
                ..
            } => {
                assert_eq!(status, 429);
                assert_eq!(retry_after, Some(60));
            }
            _ => panic!("expected HttpError for rate limit"),
        }
    }

    #[tokio::test]
    async fn handler_server_error_returns_500() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![]),
            http_error: Some(HttpError {
                status: 500,
                message: "internal error".to_string(),
                retry_after: None,
            }),
            delay: None,
        };
        let result = crate::delivery::deliver_models(&decision).await;
        match result {
            DeliveryResult::HttpError {
                status, message, ..
            } => {
                assert_eq!(status, 500);
                assert_eq!(message, "internal error");
            }
            _ => panic!("expected HttpError for server error"),
        }
    }

    #[tokio::test]
    async fn handler_models_with_delay() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![ModelEntry {
                id: "gpt-4".to_string(),
                owned_by: "openai".to_string(),
            }]),
            http_error: None,
            delay: Some(100),
        };
        let start = std::time::Instant::now();
        let result = crate::delivery::deliver_models(&decision).await;
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
    async fn handler_models_with_entries() {
        let decision = ModelsDeliveryDecision {
            models: Some(vec![
                ModelEntry {
                    id: "gpt-4".to_string(),
                    owned_by: "openai".to_string(),
                },
                ModelEntry {
                    id: "claude-3".to_string(),
                    owned_by: "anthropic".to_string(),
                },
            ]),
            http_error: None,
            delay: None,
        };
        let result = crate::delivery::deliver_models(&decision).await;
        match result {
            DeliveryResult::JsonResponse(json) => {
                let data = json["data"].as_array().unwrap();
                assert_eq!(data.len(), 2);
                assert_eq!(data[0]["id"], "gpt-4");
                assert_eq!(data[1]["id"], "claude-3");
            }
            _ => panic!("expected JsonResponse"),
        }
    }
}
