//! OpenAI protocol request parsing and response serialization.
//!
//! Handles `/v1/chat/completions` and `/v1/models` in OpenAI format.

use serde::{Deserialize, Serialize};

use crate::types::RequestFeatures;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Request → RequestFeatures
// ---------------------------------------------------------------------------

/// Extract protocol-agnostic `RequestFeatures` from an OpenAI chat request.
pub fn extract_request_features(req: &ChatCompletionRequest) -> RequestFeatures {
    RequestFeatures {
        model: req.model.clone(),
        stream: req.stream,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

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
pub fn build_chat_completion_response(model: &str) -> ChatCompletionResponse {
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

// ---------------------------------------------------------------------------
// Models response (OpenAI format)
// ---------------------------------------------------------------------------

/// OpenAI-compatible model list response.
#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    /// Always `"list"`.
    object: String,
    /// Array of model objects.
    data: Vec<ModelObject>,
}

/// A single model entry in OpenAI format.
#[derive(Debug, Serialize)]
pub struct ModelObject {
    /// Model ID (e.g. "gpt-4", "claude-3-opus-20240229").
    id: String,
    /// Always `"model"`.
    object: String,
    /// Timestamp of model creation (epoch seconds).
    created: u64,
    /// Owning organization.
    owned_by: String,
}

/// Build a placeholder model list.
///
/// Contains a mix of known and unknown model IDs so that downstream tests
/// can verify CloseClaw's filtering of unrecognized models.
pub fn build_models_response() -> ModelsResponse {
    ModelsResponse {
        object: "list".to_string(),
        data: vec![
            ModelObject {
                id: "gpt-4".to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: "openai".to_string(),
            },
            ModelObject {
                id: "gpt-3.5-turbo".to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: "openai".to_string(),
            },
            ModelObject {
                id: "claude-3-opus-20240229".to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: "anthropic".to_string(),
            },
        ],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    #[test]
    fn placeholder_models_structure() {
        let resp = build_models_response();
        assert_eq!(resp.object, "list");
        assert_eq!(resp.data.len(), 3);
        assert_eq!(resp.data[0].id, "gpt-4");
        assert_eq!(resp.data[0].object, "model");
        assert_eq!(resp.data[0].owned_by, "openai");
        assert_eq!(resp.data[2].id, "claude-3-opus-20240229");
        assert_eq!(resp.data[2].owned_by, "anthropic");
    }

    #[test]
    fn placeholder_models_json_shape() {
        let resp = build_models_response();
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "list");
        assert!(json["data"].is_array());
        assert_eq!(json["data"][0]["id"], "gpt-4");
        assert_eq!(json["data"][0]["object"], "model");
    }
}
