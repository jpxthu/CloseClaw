//! Anthropic `/v1/messages` endpoint handler.
//!
//! Parses the Anthropic messages request body via the protocol module,
//! extracts protocol-agnostic `RequestFeatures`, and returns a placeholder OK response.

use axum::Json;

use crate::protocol::anthropic::{
    build_message_response, extract_request_features, MessageRequest, MessageResponse,
};

/// Handler for POST `/v1/messages`.
///
/// Delegates request parsing and response building to the Anthropic protocol module.
/// The scenario engine (Sequence 2) will replace the placeholder with a
/// deterministic response.
pub async fn handler(Json(req): Json<MessageRequest>) -> Json<MessageResponse> {
    let _features = extract_request_features(&req);
    // TODO(Sequence 2): pass _features to scenario engine for decision
    Json(build_message_response(&req.model))
}

#[cfg(test)]
mod tests {
    use super::*;

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
