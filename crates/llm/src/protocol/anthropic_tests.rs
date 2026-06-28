//! Tests for Anthropic ChatProtocol implementation.

use reqwest::header::{HeaderMap, CONTENT_TYPE};

use crate::protocol::{AnthropicProtocol, ChatProtocol, IncomingSseStream};
use crate::types::{
    ContentBlockType, ContentDelta, InternalMessage, InternalRequest, RawContentBlock, RawSseChunk,
    StreamEvent, ToolDefinition,
};
use closeclaw_session::persistence::ReasoningLevel;

use futures::StreamExt;

fn make_request() -> InternalRequest {
    InternalRequest {
        model: "claude-3-5-sonnet-20241022".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            ..Default::default()
        }],
        temperature: 0.7,
        max_tokens: Some(1024),
        stream: false,
        extra_body: Default::default(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    }
}

fn make_sse_chunk(event_type: &str, data: &str) -> RawSseChunk {
    RawSseChunk {
        event_type: event_type.to_string(),
        data: data.to_string(),
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
    use crate::types::SystemBlock;

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
        RawContentBlock::Thinking { thinking: ref s, .. } if s == "Let me think..."
    ));
    assert!(matches!(
        resp.content_blocks[1],
        RawContentBlock::Text(ref s) if s == "Final answer."
    ));
}

#[test]
fn test_parse_response_thinking_block_with_signature() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [
            {"type": "thinking", "thinking": "Let me think...", "signature": "sig_abc123"},
            {"type": "text", "text": "Final answer."}
        ],
        "stop_reason": "end_turn"
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(matches!(
        resp.content_blocks[0],
        RawContentBlock::Thinking { thinking: ref s, signature: Some(ref sig) } if s == "Let me think..." && sig == "sig_abc123"
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

#[test]
fn test_parse_response_tool_use_block() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [
            {
                "type": "tool_use",
                "id": "toolu_01A09q90qw90lq917835lq9",
                "name": "get_weather",
                "input": {"location": "Beijing", "unit": "celsius"}
            }
        ],
        "stop_reason": "tool_use"
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    match &resp.content_blocks[0] {
        RawContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "toolu_01A09q90qw90lq917835lq9");
            assert_eq!(name, "get_weather");
            let parsed: serde_json::Value = serde_json::from_str(input).unwrap();
            assert_eq!(parsed.get("location").unwrap(), "Beijing");
            assert_eq!(parsed.get("unit").unwrap(), "celsius");
        }
        other => panic!("Expected ToolUse, got {:?}", other),
    }
}

#[test]
fn test_parse_response_tool_use_empty_input() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [
            {
                "type": "tool_use",
                "id": "toolu_empty",
                "name": "ping",
                "input": {}
            }
        ],
        "stop_reason": "tool_use"
    });

    let resp = proto.parse_response(body).unwrap();
    match &resp.content_blocks[0] {
        RawContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "toolu_empty");
            assert_eq!(name, "ping");
            assert_eq!(input, "{}");
        }
        other => panic!("Expected ToolUse, got {:?}", other),
    }
}

#[test]
fn test_parse_response_tool_result_block() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [
            {
                "type": "tool_result",
                "tool_use_id": "toolu_01A09q90qw90lq917835lq9",
                "content": "25°C, sunny"
            }
        ],
        "stop_reason": "end_turn"
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    match &resp.content_blocks[0] {
        RawContentBlock::ToolResult {
            tool_call_id,
            content,
        } => {
            assert_eq!(tool_call_id, "toolu_01A09q90qw90lq917835lq9");
            assert_eq!(content, "25°C, sunny");
        }
        other => panic!("Expected ToolResult, got {:?}", other),
    }
}

#[test]
fn test_parse_response_tool_result_array_content() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [
            {
                "type": "tool_result",
                "tool_use_id": "toolu_arr",
                "content": [
                    {"type": "text", "text": "Hello "},
                    {"type": "text", "text": "world"}
                ]
            }
        ],
        "stop_reason": "end_turn"
    });

    let resp = proto.parse_response(body).unwrap();
    match &resp.content_blocks[0] {
        RawContentBlock::ToolResult {
            tool_call_id,
            content,
        } => {
            assert_eq!(tool_call_id, "toolu_arr");
            assert_eq!(content, "Hello world");
        }
        other => panic!("Expected ToolResult, got {:?}", other),
    }
}

#[test]
fn test_parse_response_mixed_blocks() {
    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "content": [
            {"type": "thinking", "thinking": "Analyzing..."},
            {"type": "text", "text": "Here is the result."},
            {
                "type": "tool_use",
                "id": "toolu_mix",
                "name": "search",
                "input": {"q": "test"}
            },
            {
                "type": "tool_result",
                "tool_use_id": "toolu_mix",
                "content": "Found it"
            }
        ],
        "stop_reason": "end_turn"
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 4);

    assert!(matches!(
        &resp.content_blocks[0],
        RawContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &resp.content_blocks[1],
        RawContentBlock::Text(s) if s == "Here is the result."
    ));
    match &resp.content_blocks[2] {
        RawContentBlock::ToolUse { id, name, .. } => {
            assert_eq!(id, "toolu_mix");
            assert_eq!(name, "search");
        }
        other => panic!("Expected ToolUse at index 2, got {:?}", other),
    }
    match &resp.content_blocks[3] {
        RawContentBlock::ToolResult {
            tool_call_id,
            content,
        } => {
            assert_eq!(tool_call_id, "toolu_mix");
            assert_eq!(content, "Found it");
        }
        other => panic!("Expected ToolResult at index 3, got {:?}", other),
    }
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
    let proto = AnthropicProtocol::new();
    let mut headers = HeaderMap::new();
    proto.decorate_headers(&mut headers).unwrap();

    assert!(headers.get("x-api-key").is_some());
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
            ..Default::default()
        },
        InternalMessage {
            role: "assistant".to_string(),
            content: "Hey!".to_string(),
            ..Default::default()
        },
        InternalMessage {
            role: "user".to_string(),
            content: "What's up?".to_string(),
            ..Default::default()
        },
    ];
    let body = proto.build_request(&request).unwrap();

    let messages = body.get("messages").unwrap().as_array().unwrap();
    assert_eq!(messages.len(), 3);

    assert!(messages[0].get("content").unwrap().is_string());
    assert_eq!(messages[0].get("content").unwrap().as_str().unwrap(), "Hi");
    assert!(messages[1].get("content").unwrap().is_string());
    assert_eq!(
        messages[1].get("content").unwrap().as_str().unwrap(),
        "Hey!"
    );

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

    assert!(body.get("messages").unwrap().as_array().unwrap().is_empty());
}

#[test]
fn test_build_request_tools_section_cache_control() {
    use crate::types::SystemBlock;

    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    let full_static = format!("Role: You are a helpful assistant.\n\n## Tools\n\n- web_search: search the web\n- read: read files");
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
    use crate::types::SystemBlock;

    let proto = AnthropicProtocol::new();
    let mut request = make_request();
    request.messages = vec![InternalMessage {
        role: "user".to_string(),
        content: "Hello".to_string(),
        ..Default::default()
    }];
    request.system_blocks = Some(vec![SystemBlock {
        text: "System prompt".to_string(),
        cache: true,
    }]);
    let body = proto.build_request(&request).unwrap();

    let system = body.get("system").unwrap().as_array().unwrap();
    assert_eq!(system.len(), 1);
    assert_eq!(
        system[0].get("cache_control").unwrap(),
        &serde_json::json!({ "type": "ephemeral" })
    );

    let messages = body.get("messages").unwrap().as_array().unwrap();
    assert_eq!(messages.len(), 1);
    let content = messages[0].get("content").unwrap().as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(
        content[0].get("cache_control").unwrap(),
        &serde_json::json!({ "type": "ephemeral" })
    );
}

// ── parse_sse_stream tests ───────────────────────────────────────────────
#[tokio::test]
async fn test_sse_text_stream() {
    let proto = AnthropicProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(
            "message_start",
            r#"{"message":{"usage":{"input_tokens":10,"output_tokens":0}}}"#,
        ),
        make_sse_chunk(
            "content_block_start",
            r#"{"index":0,"content_block":{"type":"text"}}"#,
        ),
        make_sse_chunk(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
        ),
        make_sse_chunk(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"text_delta","text":" world"}}"#,
        ),
        make_sse_chunk("content_block_stop", r#"{"index":0}"#),
        make_sse_chunk(
            "message_delta",
            r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}"#,
        ),
        make_sse_chunk("message_stop", "{}"),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text
        }
    ));

    let evt = stream.next().await.unwrap().unwrap();
    assert!(
        matches!(evt, StreamEvent::BlockDelta { index: 0, delta: ContentDelta::Text { text } } if text == "Hello")
    );

    let evt = stream.next().await.unwrap().unwrap();
    assert!(
        matches!(evt, StreamEvent::BlockDelta { index: 0, delta: ContentDelta::Text { text } } if text == " world")
    );

    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text
        }
    ));

    let evt = stream.next().await.unwrap().unwrap();
    match evt {
        StreamEvent::MessageEnd {
            usage,
            finish_reason,
        } => {
            assert_eq!(finish_reason, Some("end_turn".to_string()));
            assert!(usage.is_some());
            let u = usage.unwrap();
            assert_eq!(u.completion_tokens, 2);
        }
        other => panic!("Expected MessageEnd, got {:?}", other),
    }

    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn test_sse_thinking_stream() {
    let proto = AnthropicProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(
            "message_start",
            r#"{"message":{"usage":{"input_tokens":5,"output_tokens":0}}}"#,
        ),
        make_sse_chunk(
            "content_block_start",
            r#"{"index":0,"content_block":{"type":"thinking"}}"#,
        ),
        make_sse_chunk(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}"#,
        ),
        make_sse_chunk(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"signature_delta","signature":"sig_abc"}}"#,
        ),
        make_sse_chunk("content_block_stop", r#"{"index":0}"#),
        make_sse_chunk("message_stop", "{}"),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    // BlockStart(Thinking)
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }
    ));

    // Thinking delta
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: ref t,
                signature: None,
            },
        } if t == "Let me think..."
    ));

    // Signature delta
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: ref t,
                signature: Some(ref sig),
            },
        } if t.is_empty() && sig == "sig_abc"
    ));

    // BlockEnd(Thinking)
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }
    ));

    // MessageEnd
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(evt, StreamEvent::MessageEnd { .. }));

    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn test_sse_tool_use_stream() {
    let proto = AnthropicProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(
            "message_start",
            r#"{"message":{"usage":{"input_tokens":10,"output_tokens":0}}}"#,
        ),
        make_sse_chunk(
            "content_block_start",
            r#"{"index":0,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_weather"}}"#,
        ),
        make_sse_chunk(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":"{\"loc"}}"#,
        ),
        make_sse_chunk(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":"ation\":\"Beijing\"}"}}"#,
        ),
        make_sse_chunk("content_block_stop", r#"{"index":0}"#),
        make_sse_chunk("message_stop", "{}"),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    // BlockStart(ToolUse)
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
        }
    ));

    // ToolUseId delta
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseId { id },
        } if id == "toolu_01"
    ));

    // ToolUseName delta
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseName { name },
        } if name == "get_weather"
    ));

    // Input chunk 1
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseInputChunk { input },
        } if input == r#"{"loc"#
    ));

    // Input chunk 2
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseInputChunk { input },
        } if input == "ation\":\"Beijing\"}"
    ));

    // BlockEnd(ToolUse)
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::ToolUse,
        }
    ));

    // MessageEnd
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(evt, StreamEvent::MessageEnd { .. }));

    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn test_sse_error_event() {
    let proto = AnthropicProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![make_sse_chunk(
        "error",
        r#"{"error":{"type":"api_error","message":"Rate limit exceeded"}}"#,
    )]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::Error {
            message
        } if message == "Rate limit exceeded"
    ));

    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn test_sse_ping_ignored() {
    let proto = AnthropicProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk("ping", "{}"),
        make_sse_chunk("message_stop", "{}"),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    // ping should be skipped, message_stop is the first event
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(evt, StreamEvent::MessageEnd { .. }));

    assert!(stream.next().await.is_none());
}

// ── tools serialization tests ────────────────────────────────────────────
fn make_tool(name: &str, cache: bool) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: format!("{name} tool"),
        input_schema: Some(serde_json::json!({ "type": "object" })),
        cache,
    }
}

fn tool_names(req: &InternalRequest) -> Vec<serde_json::Value> {
    AnthropicProtocol::new()
        .build_request(req)
        .unwrap()
        .get("tools")
        .unwrap()
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn ep() -> serde_json::Value {
    serde_json::json!({"type":"ephemeral"})
}

#[test]
fn test_build_request_tools_none() {
    assert!(AnthropicProtocol::new()
        .build_request(&make_request())
        .unwrap()
        .get("tools")
        .is_none());
}

#[test]
fn test_build_request_tools_empty() {
    let mut r = make_request();
    r.tools = Some(vec![]);
    assert!(AnthropicProtocol::new()
        .build_request(&r)
        .unwrap()
        .get("tools")
        .is_none());
}

#[test]
fn test_build_request_tools_all_cached() {
    let mut r = make_request();
    r.tools = Some(vec![make_tool("a", true), make_tool("b", true)]);
    let t = tool_names(&r);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].get("cache_control").unwrap(), &ep());
    assert_eq!(t[1].get("cache_control").unwrap(), &ep());
}

#[test]
fn test_build_request_tools_partial_cached() {
    let mut r = make_request();
    r.tools = Some(vec![make_tool("a", true), make_tool("b", false)]);
    let t = tool_names(&r);
    assert_eq!(t[0].get("cache_control").unwrap(), &ep());
    assert!(t[1].get("cache_control").is_none());
}

#[test]
fn test_build_request_tools_none_cached() {
    let mut r = make_request();
    r.tools = Some(vec![make_tool("a", false), make_tool("b", false)]);
    let t = tool_names(&r);
    assert!(t[0].get("cache_control").is_none());
    assert!(t[1].get("cache_control").is_none());
}
