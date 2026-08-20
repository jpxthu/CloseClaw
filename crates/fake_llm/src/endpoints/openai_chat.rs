//! OpenAI `/v1/chat/completions` endpoint handler.
//!
//! Parses the OpenAI chat completion request body, extracts protocol-agnostic
//! `RequestFeatures`, and returns a placeholder OK response.

use axum::Json;
use serde::{Deserialize, Serialize};

use crate::types::RequestFeatures;

/// OpenAI chat completion request body.
///
/// Implements the subset of the OpenAI Chat Completions API required for
/// protocol-level parsing. Unknown fields are silently ignored.
#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    /// Model identifier (e.g. "gpt-4", "gpt-3.5-turbo").
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<ChatMessage>,
    /// Whether to stream the response (SSE).
    #[serde(default)]
    pub stream: bool,
    /// Maximum tokens to generate.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Tool definitions available to the model.
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
    /// Stop sequences.
    #[serde(default)]
    pub stop: Option<serde_json::Value>,
}

/// A single chat message in OpenAI format.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatMessage {
    /// Role: "system", "user", "assistant", "tool".
    pub role: String,
    /// Message content (string, or array of content parts for multimodal).
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    /// Tool call ID (for tool role messages).
    #[serde(default)]
    pub tool_call_id: Option<String>,
    /// Tool calls (for assistant messages).
    #[serde(default)]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

/// Extract protocol-agnostic `RequestFeatures` from an OpenAI chat request.
pub fn extract_request_features(req: &ChatCompletionRequest) -> RequestFeatures {
    RequestFeatures {
        model: req.model.clone(),
        stream: req.stream,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
    }
}

/// Placeholder OpenAI chat completion response.
#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Serialize)]
struct Choice {
    index: u32,
    message: ResponseMessage,
    finish_reason: String,
}

#[derive(Debug, Serialize)]
struct ResponseMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

/// Build a placeholder OpenAI chat completion response for the given model.
fn build_placeholder_response(model: &str) -> ChatCompletionResponse {
    ChatCompletionResponse {
        id: "chatcmpl-placeholder".to_string(),
        object: "chat.completion".to_string(),
        created: 0,
        model: model.to_string(),
        choices: vec![Choice {
            index: 0,
            message: ResponseMessage {
                role: "assistant".to_string(),
                content: "placeholder".to_string(),
            },
            finish_reason: "stop".to_string(),
        }],
        usage: Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        },
    }
}

/// Handler for POST `/v1/chat/completions`.
///
/// Parses the OpenAI chat completion request, extracts `RequestFeatures`,
/// and returns a placeholder OK response. The scenario engine (Sequence 2)
/// will replace the placeholder with a deterministic response.
pub async fn handler(Json(req): Json<ChatCompletionRequest>) -> Json<ChatCompletionResponse> {
    let _features = extract_request_features(&req);
    // TODO(Sequence 2): pass _features to scenario engine for decision
    Json(build_placeholder_response(&req.model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_features_basic() {
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
        assert!(!features.stream);
        assert_eq!(features.max_tokens, Some(1024));
        assert_eq!(features.temperature, Some(0.7));
    }

    #[test]
    fn extract_features_streaming() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            stream: true,
            max_tokens: None,
            temperature: None,
            tools: None,
            stop: None,
        };
        let features = extract_request_features(&req);
        assert!(features.stream);
        assert_eq!(features.max_tokens, None);
    }
}
