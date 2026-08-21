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

/// A single content block in an Anthropic response.
///
/// Serialized as a flat JSON object with a `type` discriminator field.
/// Each variant maps to a different Anthropic content block type:
/// - `Text` → `type: "text"`
/// - `Thinking` → `type: "thinking"`
/// - `ToolUse` → `type: "tool_use"`
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ContentBlock {
    /// Plain text content.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },
    /// Reasoning / thinking content (hidden from the user).
    #[serde(rename = "thinking")]
    Thinking {
        /// The thinking text.
        thinking: String,
        /// Optional reasoning signature for verification.
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// Tool call request from the model.
    #[serde(rename = "tool_use")]
    ToolUse {
        /// Unique tool use identifier.
        id: String,
        /// Tool function name.
        name: String,
        /// Tool input as a JSON object.
        input: serde_json::Value,
    },
}

#[derive(Debug, Serialize)]
struct Usage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation_input_tokens: Option<u32>,
}

/// Build a placeholder Anthropic message response for the given model.
pub fn build_message_response(model: &str) -> MessageResponse {
    MessageResponse {
        id: "msg-placeholder".to_string(),
        message_type: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![ContentBlock::Text {
            text: "placeholder".to_string(),
        }],
        model: model.to_string(),
        stop_reason: Some("end_turn".to_string()),
        stop_sequence: None,
        usage: Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_input_tokens: Some(0),
            cache_creation_input_tokens: Some(0),
        },
    }
}

// ---------------------------------------------------------------------------
// Response from scenario decision
// ---------------------------------------------------------------------------

/// Build an Anthropic message response from a scenario decision.
///
/// Maps response blocks to Anthropic content block format:
/// - `reasoning` block → `Thinking` content block (with optional signature)
/// - `tool_call` block → `ToolUse` content block
/// - `text` block (and others) → `Text` content block
///
/// Placeholder content is used when response blocks are empty.
pub fn build_message_response_from_decision(decision: &ScenarioDecision) -> MessageResponse {
    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut has_tool_use = false;

    for (idx, block) in decision.response_blocks.iter().enumerate() {
        match block.block_type.as_str() {
            "reasoning" => {
                let thinking = block.reasoning.clone().unwrap_or_default();
                blocks.push(ContentBlock::Thinking {
                    thinking,
                    signature: block.signature.clone(),
                });
                // If the reasoning block also has content, add a text block.
                if let Some(ref text) = block.content {
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Text { text: text.clone() });
                    }
                }
            }
            "tool_call" => {
                has_tool_use = true;
                let name = block.tool_name.clone().unwrap_or_default();
                let input_str = block.tool_arguments.clone().unwrap_or_default();
                let input: serde_json::Value = serde_json::from_str(&input_str)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                blocks.push(ContentBlock::ToolUse {
                    id: format!("toolu_{}", idx),
                    name,
                    input,
                });
            }
            _ => {
                if let Some(ref text) = block.content {
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Text { text: text.clone() });
                    }
                }
            }
        }
    }

    let blocks = if blocks.is_empty() {
        vec![ContentBlock::Text {
            text: "placeholder".to_string(),
        }]
    } else {
        blocks
    };

    let stop_reason = if has_tool_use { "tool_use" } else { "end_turn" };

    let usage = decision.usage.as_ref().map_or_else(
        || Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_input_tokens: Some(0),
            cache_creation_input_tokens: Some(0),
        },
        |u| Usage {
            input_tokens: u.prompt_tokens.unwrap_or(0),
            output_tokens: u.completion_tokens.unwrap_or(0),
            cache_read_input_tokens: if u.cache_fields_missing {
                None
            } else {
                Some(u.cache_hit_tokens.unwrap_or(0))
            },
            cache_creation_input_tokens: if u.cache_fields_missing {
                None
            } else {
                Some(u.cache_write_tokens.unwrap_or(0))
            },
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
        stop_reason: Some(stop_reason.to_string()),
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

    #[test]
    fn response_with_thinking_block() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "thinking-test".to_string(),
            stream: false,
            response_blocks: vec![
                ResponseBlock {
                    block_type: "reasoning".to_string(),
                    content: None,
                    tool_name: None,
                    tool_arguments: None,
                    reasoning: Some("Let me think about this...".to_string()),
                    signature: Some("sig-abc".to_string()),
                },
                ResponseBlock {
                    block_type: "text".to_string(),
                    content: Some("The answer is 42.".to_string()),
                    tool_name: None,
                    tool_arguments: None,
                    reasoning: None,
                    signature: None,
                },
            ],
            http_error: None,
            delay: None,
            usage: None,
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);

        // First block: thinking
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["thinking"], "Let me think about this...");
        assert_eq!(content[0]["signature"], "sig-abc");

        // Second block: text
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "The answer is 42.");

        assert_eq!(json["stop_reason"], "end_turn");
    }

    #[test]
    fn response_with_tool_use_block() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "tool-test".to_string(),
            stream: false,
            response_blocks: vec![
                ResponseBlock {
                    block_type: "text".to_string(),
                    content: Some("Let me check.".to_string()),
                    tool_name: None,
                    tool_arguments: None,
                    reasoning: None,
                    signature: None,
                },
                ResponseBlock {
                    block_type: "tool_call".to_string(),
                    content: None,
                    tool_name: Some("get_weather".to_string()),
                    tool_arguments: Some("{\"location\": \"SF\"}".to_string()),
                    reasoning: None,
                    signature: None,
                },
            ],
            http_error: None,
            delay: None,
            usage: None,
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);

        // First block: text
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Let me check.");

        // Second block: tool_use
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["name"], "get_weather");
        assert_eq!(content[1]["input"]["location"], "SF");
        assert!(content[1]["id"].as_str().unwrap().starts_with("toolu_"));

        assert_eq!(json["stop_reason"], "tool_use");
    }

    #[test]
    fn response_thinking_only_no_text() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "think-only".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "reasoning".to_string(),
                content: None,
                tool_name: None,
                tool_arguments: None,
                reasoning: Some("hmm...".to_string()),
                signature: None,
            }],
            http_error: None,
            delay: None,
            usage: None,
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["thinking"], "hmm...");
        // No signature field when None (skip_serializing_if)
        assert!(content[0]["signature"].is_null());
        assert_eq!(json["stop_reason"], "end_turn");
    }

    #[test]
    fn response_tool_use_only() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "tool-only".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "tool_call".to_string(),
                content: None,
                tool_name: Some("search".to_string()),
                tool_arguments: Some("{}".to_string()),
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            usage: None,
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["name"], "search");
        assert_eq!(json["stop_reason"], "tool_use");
    }

    #[test]
    fn response_empty_blocks_uses_placeholder() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "empty".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some(String::new()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            usage: None,
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "placeholder");
    }

    #[test]
    fn response_cache_fields_with_values() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "cache-test".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("cached response".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            usage: Some(crate::scenario::types::UsageResponse {
                prompt_tokens: Some(100),
                completion_tokens: Some(50),
                reasoning_tokens: None,
                cache_hit_tokens: Some(80),
                cache_write_tokens: Some(20),
                cache_fields_missing: false,
            }),
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["usage"]["cache_read_input_tokens"], 80);
        assert_eq!(json["usage"]["cache_creation_input_tokens"], 20);
    }

    #[test]
    fn response_cache_fields_missing_omits_cache_tokens() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "cache-missing".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("no cache info".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            usage: Some(crate::scenario::types::UsageResponse {
                prompt_tokens: Some(100),
                completion_tokens: Some(50),
                reasoning_tokens: None,
                cache_hit_tokens: Some(80),
                cache_write_tokens: Some(20),
                cache_fields_missing: true,
            }),
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        // cache_fields_missing=true → fields must be absent from JSON
        assert!(json["usage"]["cache_read_input_tokens"].is_null());
        assert!(json["usage"]["cache_creation_input_tokens"].is_null());
    }

    #[test]
    fn response_cache_fields_default_zero() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "no-cache".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("no cache".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            usage: Some(crate::scenario::types::UsageResponse {
                prompt_tokens: Some(100),
                completion_tokens: Some(50),
                reasoning_tokens: None,
                cache_hit_tokens: None,
                cache_write_tokens: None,
                cache_fields_missing: false,
            }),
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["usage"]["cache_read_input_tokens"], 0);
        assert_eq!(json["usage"]["cache_creation_input_tokens"], 0);
    }

    #[test]
    fn response_cache_fields_no_usage() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "claude-3".to_string(),
            scenario: "no-usage".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("text".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            usage: None,
        };

        let resp = build_message_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["usage"]["cache_read_input_tokens"], 0);
        assert_eq!(json["usage"]["cache_creation_input_tokens"], 0);
    }
}
