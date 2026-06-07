//! Tests for Anthropic ChatProtocol implementation.

use reqwest::header::{HeaderMap, CONTENT_TYPE};

use crate::llm::protocol::{AnthropicProtocol, ChatProtocol};
use crate::llm::types::{InternalMessage, InternalRequest, RawContentBlock};
use crate::session::persistence::ReasoningLevel;

fn make_request() -> InternalRequest {
    InternalRequest {
        model: "claude-3-5-sonnet-20241022".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }],
        temperature: 0.7,
        max_tokens: Some(1024),
        stream: false,
        extra_body: Default::default(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    }
}

// ── build_request tests ───────────────────────────────────────────────────

#[test]
fn test_build_request_basic() {
    let proto = AnthropicProtocol::new();
    let request = make_request();
    let body = proto.build_request(&request).unwrap();

    assert_eq!(body.get("model").unwrap(), "claude-3-5-sonnet-20241022");
    assert!(body.get("messages").unwrap().is_array());
    assert_eq!(body.get("max_tokens").unwrap(), &serde_json::json!(1024));
}

#[test]
fn test_build_request_no_max_tokens() {
    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.max_tokens = None;
    let body = proto.build_request(&request).unwrap();
    assert!(body.get("max_tokens").is_none());
}

#[test]
fn test_build_request_stream_flag() {
    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.stream = true;
    let body = proto.build_request(&request).unwrap();
    assert!(body.get("stream").is_none());
}

// ── system_blocks serialization tests ─────────────────────────────────────

#[test]
fn test_build_request_system_blocks_with_cache() {
    use crate::llm::types::SystemBlock;

    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.system_blocks = Some(vec![
        SystemBlock {
            text: "You are a helpful assistant.".to_string(),
            cache: true,
        },
        SystemBlock {
            text: "Current date: 2026-01-01".to_string(),
            cache: false,
        },
    ]);
    let body = proto.build_request(&request).unwrap();

    let system = body.get("system").unwrap().as_array().unwrap();
    assert_eq!(system.len(), 2);

    let first = &system[0];
    assert_eq!(first.get("type").unwrap(), "text");
    assert_eq!(first.get("text").unwrap(), "You are a helpful assistant.");
    assert_eq!(
        first.get("cache_control"),
        Some(&serde_json::json!({ "type": "ephemeral" }))
    );

    let second = &system[1];
    assert_eq!(second.get("type").unwrap(), "text");
    assert_eq!(second.get("text").unwrap(), "Current date: 2026-01-01");
    assert!(second.get("cache_control").is_none());
}

#[test]
fn test_build_request_system_blocks_empty() {
    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.system_blocks = Some(vec![]);
    let body = proto.build_request(&request).unwrap();
    assert!(body.get("system").is_none());
}

#[test]
fn test_build_request_no_system_blocks() {
    let proto = AnthropicProtocol::new();
    let request = make_request();
    let body = proto.build_request(&request).unwrap();
    assert!(body.get("system").is_none());
}

// ── parse_response tests ──────────────────────────────────────────────────

#[test]
fn test_parse_response_normal() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [
            {"type": "text", "text": "Hello, world!"}
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 5,
            "total_tokens": 15
        }
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(matches!(
        resp.content_blocks[0],
        RawContentBlock::Text(ref s) if s == "Hello, world!"
    ));
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 5);
    assert_eq!(resp.usage.total_tokens, Some(15));
    assert_eq!(resp.finish_reason, Some("end_turn".to_string()));
}

#[test]
fn test_parse_response_thinking_block() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [
            {"type": "thinking", "thinking": "Let me think..."},
            {"type": "text", "text": "Final answer."}
        ],
        "stop_reason": "end_turn"
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(matches!(
        resp.content_blocks[0],
        RawContentBlock::Thinking(ref s) if s == "Let me think..."
    ));
    assert!(matches!(
        resp.content_blocks[1],
        RawContentBlock::Text(ref s) if s == "Final answer."
    ));
}

#[test]
fn test_parse_response_empty_content() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({ "content": [], "stop_reason": "end_turn" });
    let resp = proto.parse_response(body).unwrap();
    assert!(resp.content_blocks.is_empty());
    assert_eq!(resp.usage.prompt_tokens, 0);
    assert_eq!(resp.usage.completion_tokens, 0);
}

#[test]
fn test_parse_response_missing_usage_defaults() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [{"type": "text", "text": "Hi"}],
        "stop_reason": "end_turn"
    });
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.usage.prompt_tokens, 0);
    assert_eq!(resp.usage.completion_tokens, 0);
    assert!(resp.usage.total_tokens.is_none());
}

// ── cache usage parsing tests ─────────────────────────────────────────────

#[test]
fn test_parse_usage_cache_fields() {
    let body = serde_json::json!({
        "content": [{"type": "text", "text": "hi"}],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_input_tokens": 80,
            "cache_creation_input_tokens": 20
        }
    });
    let proto = AnthropicProtocol::new();
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.usage.cache_read_tokens, Some(80));
    assert_eq!(resp.usage.cache_write_tokens, Some(20));
}

#[test]
fn test_parse_usage_no_cache_fields() {
    let body = serde_json::json!({
        "content": [{"type": "text", "text": "hi"}],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50
        }
    });
    let proto = AnthropicProtocol::new();
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.usage.cache_read_tokens, None);
    assert_eq!(resp.usage.cache_write_tokens, None);
}

// ── decorate_headers tests ────────────────────────────────────────────────

#[test]
fn test_decorate_headers_api_key() {
    std::env::remove_var("ANTHROPIC_API_KEY");
    let proto = AnthropicProtocol::new();
    let mut headers = HeaderMap::new();
    proto.decorate_headers(&mut headers).unwrap();

    let key_header = headers.get("x-api-key").unwrap();
    assert_eq!(key_header.to_str().unwrap(), "");
}

#[test]
fn test_decorate_headers_anthropic_version() {
    let proto = AnthropicProtocol::new();
    let mut headers = HeaderMap::new();
    proto.decorate_headers(&mut headers).unwrap();

    assert_eq!(
        headers.get("anthropic-version").unwrap().to_str().unwrap(),
        "2023-06-01"
    );
}

#[test]
fn test_decorate_headers_content_type() {
    let proto = AnthropicProtocol::new();
    let mut headers = HeaderMap::new();
    proto.decorate_headers(&mut headers).unwrap();

    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

// ── reasoning_level non-injection tests ─────────────────────────────────

#[test]
fn test_build_request_does_not_inject_reasoning_level_low() {
    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Low;
    let body = proto.build_request(&request).unwrap();
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("reasoning_level").is_none());
}

#[test]
fn test_build_request_does_not_inject_reasoning_level_medium() {
    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Medium;
    let body = proto.build_request(&request).unwrap();
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("reasoning_level").is_none());
}

#[test]
fn test_build_request_does_not_inject_reasoning_level_max() {
    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Max;
    let body = proto.build_request(&request).unwrap();
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("reasoning_level").is_none());
}

#[test]
fn test_build_request_high_reasoning_level_no_injection() {
    let proto = AnthropicProtocol::new();
    let request = make_request(); // default is High
    let body = proto.build_request(&request).unwrap();
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("reasoning_level").is_none());
}

// ── messages cache_control tests ──────────────────────────────────────────

#[test]
fn test_messages_cache_control_single_message() {
    let proto = AnthropicProtocol::new();
    let request = make_request(); // single "Hello" message
    let body = proto.build_request(&request).unwrap();

    let messages = body.get("messages").unwrap().as_array().unwrap();
    assert_eq!(messages.len(), 1);

    // Content should be a content blocks array with cache_control
    let content = messages[0].get("content").unwrap().as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0].get("type").unwrap(), "text");
    assert_eq!(content[0].get("text").unwrap(), "Hello");
    assert_eq!(
        content[0].get("cache_control").unwrap(),
        &serde_json::json!({ "type": "ephemeral" })
    );
}

#[test]
fn test_messages_cache_control_multiple_messages() {
    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.messages = vec![
        InternalMessage {
            role: "user".to_string(),
            content: "Hi".to_string(),
        },
        InternalMessage {
            role: "assistant".to_string(),
            content: "Hey!".to_string(),
        },
        InternalMessage {
            role: "user".to_string(),
            content: "What's up?".to_string(),
        },
    ];
    let body = proto.build_request(&request).unwrap();

    let messages = body.get("messages").unwrap().as_array().unwrap();
    assert_eq!(messages.len(), 3);

    // First two messages should keep string content
    assert!(messages[0].get("content").unwrap().is_string());
    assert_eq!(messages[0].get("content").unwrap().as_str().unwrap(), "Hi");
    assert!(messages[1].get("content").unwrap().is_string());
    assert_eq!(
        messages[1].get("content").unwrap().as_str().unwrap(),
        "Hey!"
    );

    // Last message should be content blocks with cache_control
    let last_content = messages[2].get("content").unwrap().as_array().unwrap();
    assert_eq!(last_content.len(), 1);
    assert_eq!(last_content[0].get("type").unwrap(), "text");
    assert_eq!(last_content[0].get("text").unwrap(), "What's up?");
    assert_eq!(
        last_content[0].get("cache_control").unwrap(),
        &serde_json::json!({ "type": "ephemeral" })
    );
}

#[test]
fn test_messages_cache_control_empty_messages() {
    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.messages = vec![];
    let body = proto.build_request(&request).unwrap();

    // Empty messages should produce an empty array with no cache_control added
    let messages = body.get("messages").unwrap().as_array().unwrap();
    assert!(messages.is_empty());
}

/// Verify that ToolsSection content in `system_blocks` produces `cache_control`
/// in the Anthropic request body, confirming the prefix-cache path covers tool
/// definitions embedded in the static layer.
#[test]
fn test_build_request_tools_section_cache_control() {
    use crate::llm::types::SystemBlock;

    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    // Simulate a system prompt with role section followed by ToolsSection content
    let system_static = "Role: You are a helpful assistant.";
    let tools_section = "\n\n## Tools\n\n- web_search: search the web\n- read: read files";
    let full_static = format!("{system_static}{tools_section}");
    request.system_blocks = Some(vec![SystemBlock {
        text: full_static,
        cache: true,
    }]);
    let body = proto.build_request(&request).unwrap();

    let system = body.get("system").unwrap().as_array().unwrap();
    assert_eq!(system.len(), 1);
    let block = &system[0];
    assert_eq!(block.get("type").unwrap(), "text");
    assert!(
        block
            .get("text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("web_search"),
        "system block should contain ToolsSection content"
    );
    assert_eq!(
        block.get("cache_control").unwrap(),
        &serde_json::json!({ "type": "ephemeral" }),
        "ToolsSection block must have cache_control when cache=true"
    );
}

#[test]
fn test_messages_cache_control_with_system_blocks() {
    use crate::llm::types::SystemBlock;

    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.messages = vec![InternalMessage {
        role: "user".to_string(),
        content: "Hello".to_string(),
    }];
    request.system_blocks = Some(vec![SystemBlock {
        text: "System prompt".to_string(),
        cache: true,
    }]);
    let body = proto.build_request(&request).unwrap();

    // System should have cache_control
    let system = body.get("system").unwrap().as_array().unwrap();
    assert_eq!(system.len(), 1);
    assert_eq!(
        system[0].get("cache_control").unwrap(),
        &serde_json::json!({ "type": "ephemeral" })
    );

    // Messages should also have cache_control on the last message
    let messages = body.get("messages").unwrap().as_array().unwrap();
    assert_eq!(messages.len(), 1);
    let content = messages[0].get("content").unwrap().as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(
        content[0].get("cache_control").unwrap(),
        &serde_json::json!({ "type": "ephemeral" })
    );
}
