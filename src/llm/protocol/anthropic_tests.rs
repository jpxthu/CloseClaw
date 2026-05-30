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
