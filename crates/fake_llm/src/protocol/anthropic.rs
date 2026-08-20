//! Anthropic protocol request parsing and response serialization.
//!
//! Handles `/v1/messages` in Anthropic format.

use serde::{Deserialize, Serialize};

use crate::scenario::types::MessageEntry;
use crate::types::{
    extract_text_from_content, extract_tool_names, RequestFeatures, ScenarioDecision,
};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Anthropic messages request body.
///
/// Implements the subset of the Anthropic Messages API required for
/// protocol-level parsing. Unknown fields are silently ignored.
#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    /// Model identifier (e.g. "claude-3-opus-20240229").
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// System prompt (string or array of content blocks).
    #[serde(default)]
    pub system: Option<serde_json::Value>,
    /// Whether to stream the response (SSE events).
    #[serde(default)]
    pub stream: bool,
    /// Sampling temperature.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Tool definitions available to the model.
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
    /// Stop sequences.
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    /// Metadata for request tracking.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// A single message in Anthropic format.
///
/// Messages alternate between `user` and `assistant` roles. Content can be
/// a simple string or an array of content blocks (text, images, tool_use, etc.).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    /// Role: "user" or "assistant".
    pub role: String,
    /// Message content — string or array of content blocks.
    pub content: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Request → RequestFeatures
// ---------------------------------------------------------------------------

/// Extract protocol-agnostic `RequestFeatures` from an Anthropic message request.
pub fn extract_request_features(req: &MessageRequest) -> RequestFeatures {
    let messages: Vec<MessageEntry> = req
        .messages
        .iter()
        .map(|m| {
            let content = extract_text_from_content(&m.content);
            MessageEntry {
                role: m.role.clone(),
                content,
            }
        })
        .collect();

    let tools = req
        .tools
        .as_ref()
        .map(|ts| extract_tool_names(ts))
        .unwrap_or_default();

    RequestFeatures {
        model: req.model.clone(),
        stream: req.stream,
        max_tokens: Some(req.max_tokens),
        temperature: req.temperature,
        messages,
        tools,
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Placeholder Anthropic message response.
#[derive(Debug, Serialize)]
pub struct MessageResponse {
    id: String,
    #[serde(rename = "type")]
    message_type: String,
    role: String,
    content: Vec<ContentBlock>,
    model: String,
    stop_reason: Option<String>,
    stop_sequence: Option<String>,
    usage: Usage,
}

#[derive(Debug, Serialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct Usage {
    input_tokens: u32,
    output_tokens: u32,
}

/// Build a placeholder Anthropic message response for the given model.
pub fn build_message_response(model: &str) -> MessageResponse {
    MessageResponse {
        id: "msg-placeholder".to_string(),
        message_type: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![ContentBlock {
            block_type: "text".to_string(),
            text: "placeholder".to_string(),
        }],
        model: model.to_string(),
        stop_reason: Some("end_turn".to_string()),
        stop_sequence: None,
        usage: Usage {
            input_tokens: 0,
            output_tokens: 0,
        },
    }
}

// ---------------------------------------------------------------------------
// Response from scenario decision
// ---------------------------------------------------------------------------

/// Build an Anthropic message response from a scenario decision.
///
/// Maps response blocks to Anthropic content block format and includes
/// optional usage fields. Placeholder content is used when response blocks
/// are empty.
pub fn build_message_response_from_decision(decision: &ScenarioDecision) -> MessageResponse {
    let blocks: Vec<ContentBlock> = decision
        .response_blocks
        .iter()
        .map(|b| ContentBlock {
            block_type: "text".to_string(),
            text: b.content.clone().unwrap_or_default(),
        })
        .collect();

    let blocks = if blocks.is_empty() {
        vec![ContentBlock {
            block_type: "text".to_string(),
            text: "placeholder".to_string(),
        }]
    } else {
        blocks
    };

    let usage = decision.usage.as_ref().map_or_else(
        || Usage {
            input_tokens: 0,
            output_tokens: 0,
        },
        |u| Usage {
            input_tokens: u.prompt_tokens.unwrap_or(0),
            output_tokens: u.completion_tokens.unwrap_or(0),
        },
    );

    MessageResponse {
        id: format!(
            "msg-{}",
            &decision.scenario[..8.min(decision.scenario.len())]
        ),
        message_type: "message".to_string(),
        role: "assistant".to_string(),
        content: blocks,
        model: decision.model.clone(),
        stop_reason: Some("end_turn".to_string()),
        stop_sequence: None,
        usage,
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
        assert!(!features.stream);
        assert_eq!(features.max_tokens, Some(1024));
        assert_eq!(features.temperature, Some(0.7));
        assert!(features.messages.is_empty());
        assert!(features.tools.is_empty());
    }

    #[test]
    fn extract_features_streaming() {
        let req = MessageRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            messages: vec![],
            max_tokens: 512,
            system: Some(serde_json::json!("You are helpful.")),
            stream: true,
            temperature: None,
            tools: None,
            stop_sequences: None,
            metadata: None,
        };
        let features = extract_request_features(&req);
        assert!(features.stream);
        assert_eq!(features.max_tokens, Some(512));
        assert_eq!(features.temperature, None);
        assert!(features.messages.is_empty());
        assert!(features.tools.is_empty());
    }

    #[test]
    fn extract_features_with_tools() {
        let req = MessageRequest {
            model: "claude-3-opus-20240229".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: serde_json::json!("Hello"),
            }],
            max_tokens: 2048,
            system: None,
            stream: false,
            temperature: Some(0.0),
            tools: Some(vec![serde_json::json!({
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": { "type": "object", "properties": {} }
            })]),
            stop_sequences: Some(vec!["\n\n".to_string()]),
            metadata: None,
        };
        let features = extract_request_features(&req);
        assert_eq!(features.model, "claude-3-opus-20240229");
        assert!(!features.stream);
        assert_eq!(features.max_tokens, Some(2048));
        assert_eq!(features.temperature, Some(0.0));
        assert_eq!(features.messages.len(), 1);
        assert_eq!(features.messages[0].role, "user");
        assert_eq!(features.messages[0].content, "Hello");
        assert_eq!(features.tools, vec!["get_weather"]);
    }
}
