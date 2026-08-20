//! OpenAI `/v1/chat/completions` endpoint handler.
//!
//! Parses the OpenAI chat completion request body via the protocol module,
//! extracts protocol-agnostic `RequestFeatures`, and returns a placeholder OK response.

use axum::Json;

use crate::protocol::openai::{
    build_chat_completion_response, extract_request_features, ChatCompletionRequest,
    ChatCompletionResponse,
};

/// Handler for POST `/v1/chat/completions`.
///
/// Delegates request parsing and response building to the OpenAI protocol module.
/// The scenario engine (Sequence 2) will replace the placeholder with a
/// deterministic response.
pub async fn handler(Json(req): Json<ChatCompletionRequest>) -> Json<ChatCompletionResponse> {
    let _features = extract_request_features(&req);
    // TODO(Sequence 2): pass _features to scenario engine for decision
    Json(build_chat_completion_response(&req.model))
}

#[cfg(test)]
mod tests {
    use super::*;

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
