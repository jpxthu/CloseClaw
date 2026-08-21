//! OpenAI protocol request parsing and response serialization.
//!
//! Handles `/v1/chat/completions` and `/v1/models` in OpenAI format.

use serde::{Deserialize, Serialize};

use crate::scenario::types::MessageEntry;
use crate::types::{
    extract_text_from_content, extract_tool_names, RequestFeatures, ScenarioDecision,
};

// ---------------------------------------------------------------------------
// Cache token details
// ---------------------------------------------------------------------------

/// Breakdown of prompt token details for cache information.
#[derive(Debug, Serialize)]
struct PromptTokensDetails {
    /// Number of cached tokens that were a cache hit.
    cached_tokens: u32,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// OpenAI chat completion request body.
///
/// Implements the subset of the OpenAI Chat Completions API required for
/// protocol-level parsing. Unknown fields are silently ignored.
/// Stream options for controlling streaming behavior.
///
/// When present, the client signals whether to include token usage
/// in the final streaming chunk.
#[derive(Debug, Deserialize)]
pub struct StreamOptions {
    /// Whether to include usage information in the final streaming chunk.
    pub include_usage: bool,
}

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
    /// Stream options controlling streaming behavior (e.g. usage inclusion).
    #[serde(default)]
    pub stream_options: Option<StreamOptions>,
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
    let messages: Vec<MessageEntry> = req
        .messages
        .iter()
        .map(|m| {
            let content = match &m.content {
                Some(v) => extract_text_from_content(v),
                None => String::new(),
            };
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
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        messages,
        tools,
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
    /// Reasoning content (for models that expose hidden thinking).
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    /// Tool calls requested by the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
}

/// A single tool call in OpenAI format.
#[derive(Debug, Serialize)]
struct ToolCall {
    /// Unique tool call identifier.
    id: String,
    /// Always "function".
    #[serde(rename = "type")]
    call_type: String,
    /// Function name and arguments.
    function: ToolCallFunction,
    /// Index of this tool call in the array.
    index: u32,
}

/// Function invocation details within a tool call.
#[derive(Debug, Serialize)]
struct ToolCallFunction {
    /// Function name.
    name: String,
    /// Arguments as a JSON string.
    arguments: String,
}

#[derive(Debug, Serialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_tokens_details: Option<PromptTokensDetails>,
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
                reasoning_content: None,
                tool_calls: None,
            },
            finish_reason: "stop".to_string(),
        }],
        usage: Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Response from scenario decision
// ---------------------------------------------------------------------------

/// Build an OpenAI chat completion response from a scenario decision.
///
/// Maps response blocks to OpenAI content format and includes optional
/// usage fields. Handles reasoning blocks (`reasoning_content`), tool call
/// blocks (`tool_calls` array), and text blocks (`content`).
pub fn build_chat_completion_response_from_decision(
    decision: &ScenarioDecision,
) -> ChatCompletionResponse {
    let mut content = String::new();
    let mut reasoning_content: Option<String> = None;
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for (idx, block) in decision.response_blocks.iter().enumerate() {
        match block.block_type.as_str() {
            "reasoning" => {
                reasoning_content = block.reasoning.clone();
                if let Some(ref text) = block.content {
                    if !text.is_empty() {
                        content.push_str(text);
                    }
                }
            }
            "tool_call" => {
                let name = block.tool_name.clone().unwrap_or_default();
                let args = block.tool_arguments.clone().unwrap_or_default();
                tool_calls.push(ToolCall {
                    id: format!("call_{}", idx),
                    call_type: "function".to_string(),
                    function: ToolCallFunction {
                        name,
                        arguments: args,
                    },
                    index: tool_calls.len() as u32,
                });
            }
            _ => {
                if let Some(ref text) = block.content {
                    if !text.is_empty() {
                        content.push_str(text);
                    }
                }
            }
        }
    }

    let content = if content.is_empty() && tool_calls.is_empty() {
        "placeholder".to_string()
    } else {
        content
    };

    let finish_reason = if tool_calls.is_empty() {
        "stop".to_string()
    } else {
        "tool_calls".to_string()
    };

    let usage = build_usage_from_decision(decision);

    ChatCompletionResponse {
        id: format!(
            "chatcmpl-{}",
            &decision.scenario[..8.min(decision.scenario.len())]
        ),
        object: "chat.completion".to_string(),
        created: 0,
        model: decision.model.clone(),
        choices: vec![Choice {
            index: 0,
            message: ResponseMessage {
                role: "assistant".to_string(),
                content,
                reasoning_content: reasoning_content.filter(|s| !s.is_empty()),
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
            },
            finish_reason,
        }],
        usage,
    }
}

/// Build OpenAI usage from a scenario decision.
fn build_usage_from_decision(decision: &ScenarioDecision) -> Usage {
    decision.usage.as_ref().map_or_else(
        || Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
        },
        |u| {
            let prompt = u.prompt_tokens.unwrap_or(0);
            let completion = u.completion_tokens.unwrap_or(0);
            let prompt_tokens_details = u
                .cache_hit_tokens
                .filter(|&n| n > 0)
                .map(|n| PromptTokensDetails { cached_tokens: n });
            Usage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: prompt + completion,
                prompt_tokens_details,
            }
        },
    )
}

// ---------------------------------------------------------------------------
// Models response (OpenAI format)
// ---------------------------------------------------------------------------

/// OpenAI-compatible model list response.
#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    /// Always `"list"`.
    pub(crate) object: String,
    /// Array of model objects.
    pub(crate) data: Vec<ModelObject>,
}

/// A single model entry in OpenAI format.
#[derive(Debug, Serialize)]
pub struct ModelObject {
    /// Model ID (e.g. "gpt-4", "claude-3-opus-20240229").
    pub(crate) id: String,
    /// Always `"model"`.
    pub(crate) object: String,
    /// Timestamp of model creation (epoch seconds).
    pub(crate) created: u64,
    /// Owning organization.
    pub(crate) owned_by: String,
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
            stream_options: None,
        };
        let features = extract_request_features(&req);
        assert_eq!(features.model, "gpt-4");
        assert!(!features.stream);
        assert_eq!(features.max_tokens, Some(1024));
        assert_eq!(features.temperature, Some(0.7));
        assert!(features.messages.is_empty());
        assert!(features.tools.is_empty());
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
            stream_options: None,
        };
        let features = extract_request_features(&req);
        assert!(features.stream);
        assert_eq!(features.max_tokens, None);
        assert!(features.messages.is_empty());
        assert!(features.tools.is_empty());
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

    #[test]
    fn response_with_reasoning_content() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "reasoning-test".to_string(),
            stream: false,
            response_blocks: vec![
                ResponseBlock {
                    block_type: "reasoning".to_string(),
                    content: None,
                    tool_name: None,
                    tool_arguments: None,
                    reasoning: Some("Let me think...".to_string()),
                    signature: None,
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
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };

        let resp = build_chat_completion_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(
            json["choices"][0]["message"]["content"],
            "The answer is 42."
        );
        assert_eq!(
            json["choices"][0]["message"]["reasoning_content"],
            "Let me think..."
        );
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn response_with_tool_calls() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "gpt-4".to_string(),
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
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };

        let resp = build_chat_completion_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["choices"][0]["message"]["content"], "Let me check.");
        assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
        let calls = json["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_1");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[0]["function"]["name"], "get_weather");
        assert_eq!(calls[0]["function"]["arguments"], "{\"location\": \"SF\"}");
        assert_eq!(calls[0]["index"], 0);
    }

    #[test]
    fn response_with_reasoning_only_no_content() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "reasoning-only".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "reasoning".to_string(),
                content: None,
                tool_name: None,
                tool_arguments: None,
                reasoning: Some("thinking...".to_string()),
                signature: None,
            }],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };

        let resp = build_chat_completion_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        // Empty content falls back to placeholder
        assert_eq!(json["choices"][0]["message"]["content"], "placeholder");
        assert_eq!(
            json["choices"][0]["message"]["reasoning_content"],
            "thinking..."
        );
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn usage_with_cache_hit_tokens() {
        use crate::scenario::types::{ResponseBlock, UsageResponse};

        let decision = ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "cache-hit".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("hello".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: Some(UsageResponse {
                prompt_tokens: Some(100),
                completion_tokens: Some(20),
                cache_hit_tokens: Some(50),
                cache_write_tokens: None,
                ..Default::default()
            }),
        };

        let resp = build_chat_completion_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["usage"]["prompt_tokens_details"]["cached_tokens"], 50);
    }

    #[test]
    fn usage_without_cache_hit_tokens() {
        use crate::scenario::types::{ResponseBlock, UsageResponse};

        let decision = ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "no-cache".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("hello".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: Some(UsageResponse {
                prompt_tokens: Some(100),
                completion_tokens: Some(20),
                cache_hit_tokens: None,
                cache_write_tokens: None,
                ..Default::default()
            }),
        };

        let resp = build_chat_completion_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        assert!(json["usage"].get("prompt_tokens_details").is_none());
    }

    #[test]
    fn usage_with_cache_hit_zero() {
        use crate::scenario::types::{ResponseBlock, UsageResponse};

        let decision = ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "cache-zero".to_string(),
            stream: false,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("hello".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: Some(UsageResponse {
                prompt_tokens: Some(100),
                completion_tokens: Some(20),
                cache_hit_tokens: Some(0),
                cache_write_tokens: None,
                ..Default::default()
            }),
        };

        let resp = build_chat_completion_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        // cache_hit_tokens == 0 should not appear in JSON
        assert!(json["usage"].get("prompt_tokens_details").is_none());
    }

    #[test]
    fn response_multiple_tool_calls() {
        use crate::scenario::types::ResponseBlock;

        let decision = ScenarioDecision {
            model: "gpt-4".to_string(),
            scenario: "multi-tool".to_string(),
            stream: false,
            response_blocks: vec![
                ResponseBlock {
                    block_type: "tool_call".to_string(),
                    content: None,
                    tool_name: Some("search".to_string()),
                    tool_arguments: Some("{}".to_string()),
                    reasoning: None,
                    signature: None,
                },
                ResponseBlock {
                    block_type: "tool_call".to_string(),
                    content: None,
                    tool_name: Some("calc".to_string()),
                    tool_arguments: Some("{}".to_string()),
                    reasoning: None,
                    signature: None,
                },
            ],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        };

        let resp = build_chat_completion_response_from_decision(&decision);
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
        let calls = json["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["function"]["name"], "search");
        assert_eq!(calls[0]["index"], 0);
        assert_eq!(calls[1]["function"]["name"], "calc");
        assert_eq!(calls[1]["index"], 1);
    }

    #[test]
    fn stream_options_with_include_usage_true() {
        let json_str = r#"{
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
            "stream_options": {"include_usage": true}
        }"#;
        let req: ChatCompletionRequest = serde_json::from_str(json_str).unwrap();
        assert!(req.stream);
        let opts = req.stream_options.unwrap();
        assert!(opts.include_usage);
    }

    #[test]
    fn stream_options_with_include_usage_false() {
        let json_str = r#"{
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
            "stream_options": {"include_usage": false}
        }"#;
        let req: ChatCompletionRequest = serde_json::from_str(json_str).unwrap();
        assert!(req.stream);
        let opts = req.stream_options.unwrap();
        assert!(!opts.include_usage);
    }

    #[test]
    fn stream_options_absent() {
        let json_str = r#"{
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }"#;
        let req: ChatCompletionRequest = serde_json::from_str(json_str).unwrap();
        assert!(req.stream);
        assert!(req.stream_options.is_none());
    }

    #[test]
    fn stream_options_backward_compatible_no_field() {
        // Old request format without stream_options should still parse
        let json_str = r#"{
            "model": "gpt-3.5-turbo",
            "messages": [],
            "max_tokens": 512
        }"#;
        let req: ChatCompletionRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req.model, "gpt-3.5-turbo");
        assert!(!req.stream);
        assert!(req.stream_options.is_none());
    }

    #[test]
    fn extract_features_with_stream_options() {
        // Verify that the stream_options field is accessible on the request
        // and can be used to determine include_usage for DeliveryConfig.
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            stream: true,
            max_tokens: None,
            temperature: None,
            tools: None,
            stop: None,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
        };
        // The stream_options are not part of RequestFeatures (protocol-agnostic),
        // but are accessible on the request struct for the endpoint handler.
        assert!(req.stream_options.as_ref().unwrap().include_usage);

        // Verify feature extraction still works correctly
        let features = extract_request_features(&req);
        assert!(features.stream);
    }
}
