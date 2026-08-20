//! OpenAI `/v1/chat/completions` endpoint handler.
//!
//! Parses the OpenAI chat completion request body via the protocol module,
//! extracts protocol-agnostic `RequestFeatures`, delegates to the scenario
//! engine, and returns the appropriate response.

use axum::{extract::State, http::StatusCode, Json};

use crate::protocol::openai::{
    build_chat_completion_response_from_decision, extract_request_features, ChatCompletionRequest,
    ChatCompletionResponse,
};
use crate::scenario::{DecisionOutcome, ScenarioState};

/// Handler for POST `/v1/chat/completions`.
///
/// Extracts request features, delegates to the scenario engine via shared state,
/// and builds the response per the engine's decision.
pub async fn handler(
    State(state): State<ScenarioState>,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletionResponse>, (StatusCode, String)> {
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
        DecisionOutcome::Decision(decision) => Ok(Json(
            build_chat_completion_response_from_decision(&decision),
        )),
    }
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
