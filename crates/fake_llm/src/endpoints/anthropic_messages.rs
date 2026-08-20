//! Anthropic `/v1/messages` endpoint handler.
//!
//! Parses the Anthropic messages request body via the protocol module,
//! extracts protocol-agnostic `RequestFeatures`, delegates to the scenario
//! engine, and returns the appropriate response.

use axum::{extract::State, http::StatusCode, Json};

use crate::protocol::anthropic::{
    build_message_response_from_decision, extract_request_features, MessageRequest, MessageResponse,
};
use crate::scenario::{DecisionOutcome, ScenarioState};

/// Handler for POST `/v1/messages`.
///
/// Extracts request features, delegates to the scenario engine via shared state,
/// and builds the response per the engine's decision.
pub async fn handler(
    State(state): State<ScenarioState>,
    Json(req): Json<MessageRequest>,
) -> Result<Json<MessageResponse>, (StatusCode, String)> {
    let features = extract_request_features(&req);

    let outcome = {
        let mut engine = state
            .engine
            .lock()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        engine.decide(&features)
    };

    match outcome {
        DecisionOutcome::Error(e) => {
            let status =
                StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            Err((status, e.message))
        }
        DecisionOutcome::Decision(decision) => {
            Ok(Json(build_message_response_from_decision(&decision)))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::anthropic::{
        build_message_response, extract_request_features, MessageRequest,
    };

    #[test]
    fn handler_delegates_to_protocol() {
        // Verify the endpoint handler compiles and can invoke protocol functions.
        let req = MessageRequest {
            model: "claude-3-opus-20240229".to_string(),
            messages: vec![],
            max_tokens: 1024,
            system: None,
            stream: false,
            temperature: Some(0.7),
            tools: None,
            stop_sequences: None,
            metadata: None,
        };
        let features = extract_request_features(&req);
        assert_eq!(features.model, "claude-3-opus-20240229");

        // Verify response serializes to valid JSON (fields are private, test via serde)
        let resp = build_message_response(&req.model);
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["type"], "message");
        assert_eq!(json["role"], "assistant");
    }
}
