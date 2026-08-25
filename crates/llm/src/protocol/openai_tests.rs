//! Tests for OpenAI protocol — extracted to stay under 500-line limit.
use super::{
    ChatProtocol, ContentBlockType, ContentDelta, IncomingSseStream, InternalRequest,
    OpenAiProtocol, StreamEvent,
};
use crate::types::{RawContentBlock, RawSseChunk};
use futures::StreamExt;

use closeclaw_session::persistence::ReasoningLevel;

fn make_request() -> InternalRequest {
    InternalRequest {
        model: "gpt-4".to_string(),
        messages: vec![super::InternalMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            ..Default::default()
        }],
        temperature: 0.7,
        max_tokens: Some(256),
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

fn make_sse_chunk(data: &str) -> RawSseChunk {
    RawSseChunk {
        event_type: "message".to_string(),
        data: data.to_string(),
    }
}

#[tokio::test]
async fn test_parse_sse_tool_calls_basic() {
    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{\"location\""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":": \"Beijing\"}"}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"}"}}]}}]}"#,
        ),
        make_sse_chunk(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    // BlockStart(ToolUse)
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockStart {
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));

    // ToolUseId
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::ToolUseId { id }, .. } if id == "call_abc"
    ));

    // ToolUseName
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::ToolUseName { name }, .. } if name == "get_weather"
    ));

    // ToolUseInputChunk 1: {"location"
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == r#"{"location""#
    ));

    // ToolUseInputChunk 2: : "Beijing"}
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == ": \"Beijing\"}"
    ));

    // ToolUseInputChunk 3: }
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == "}"
    ));

    // BlockEnd(ToolUse)
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockEnd {
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));

    // MessageEnd
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(evt, StreamEvent::MessageEnd { .. }));

    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn test_parse_sse_text_then_tool_calls() {
    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(r#"{"choices":[{"delta":{"content":"Thinking..."}}]}"#),
        make_sse_chunk(r#"{"choices":[{"delta":{"content":" here's a tool call."}}]}"#),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"\"query\""}}]}}]}"#,
        ),
        make_sse_chunk(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    // Text BlockStart
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockStart {
            block_type: ContentBlockType::Text,
            ..
        }
    ));

    // Text content 1
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::Text { text }, .. } if text == "Thinking..."
    ));

    // Text content 2
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::Text { text }, .. } if text == " here's a tool call."
    ));

    // Text BlockEnd
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockEnd {
            block_type: ContentBlockType::Text,
            ..
        }
    ));

    // ToolUse BlockStart
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockStart {
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));

    // ToolUseId
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::ToolUseId { id }, .. } if id == "call_1"
    ));

    // ToolUseName
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::ToolUseName { name }, .. } if name == "search"
    ));

    // ToolUseInputChunk
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta { delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == "\"query\""
    ));

    // ToolUse BlockEnd
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockEnd {
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));

    // MessageEnd
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(evt, StreamEvent::MessageEnd { .. }));

    assert!(stream.next().await.is_none());
}

#[test]
fn test_parse_response_with_reasoning_content() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "",
                "reasoning_content": "Let me think about this..."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // Empty content + reasoning_content → single Text block (reasoning_content as Text)
    assert_eq!(resp.content_blocks.len(), 1);
    let RawContentBlock::Text(text) = &resp.content_blocks[0] else {
        panic!("expected Text block");
    };
    assert_eq!(text, "Let me think about this...");
}

#[test]
fn test_parse_response_with_both_content_and_reasoning() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "The answer is 42.",
                "reasoning_content": "Let me think about this..."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // Both content and reasoning → Text + Thinking (independent)
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "The answer is 42.")
    );
    assert!(
        matches!(&resp.content_blocks[1], RawContentBlock::Thinking { thinking, signature: None }
            if thinking == "Let me think about this...")
    );
}

#[test]
fn test_parse_response_both_content_and_reasoning_empty() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "",
                "reasoning_content": null
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 0,
            "total_tokens": 100
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // Both empty → single empty Text block
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()));
}

#[test]
fn test_parse_response_no_reasoning_content() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Hello!"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // No reasoning_content → only Text block
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Hello!"));
}

#[test]
fn test_parse_response_reasoning_as_text_when_content_empty() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "reasoning_content": "Deep reasoning here."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // content=null + reasoning_content non-empty → single Text block with reasoning content
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Deep reasoning here.")
    );
}

#[test]
fn test_parse_response_thinking_then_text_order() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "The answer is 42.",
                "reasoning_content": "Let me think about this..."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // content + reasoning_content → Text + Thinking (independent)
    assert_eq!(resp.content_blocks.len(), 2);
    match &resp.content_blocks[0] {
        RawContentBlock::Text(text) => {
            assert_eq!(text, "The answer is 42.");
        }
        _ => panic!("Expected Text block"),
    }
    match &resp.content_blocks[1] {
        RawContentBlock::Thinking {
            thinking,
            signature: None,
        } => {
            assert_eq!(thinking, "Let me think about this...");
        }
        _ => panic!("Expected Thinking block"),
    }
}

// ── reasoning_effort is NOT injected by protocol layer ───────────────────────
// reasoning_effort is injected by DeepSeekPlugin via extra_body, not by the protocol.
// These tests verify the protocol layer does not inject reasoning_effort directly.

#[test]
fn test_build_request_does_not_inject_reasoning_effort_low() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Low;
    let body = proto.build_request(&request).unwrap();
    assert!(
        body.get("reasoning_effort").is_none(),
        "protocol layer should not inject reasoning_effort"
    );
}

#[test]
fn test_build_request_does_not_inject_reasoning_effort_medium() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Medium;
    let body = proto.build_request(&request).unwrap();
    assert!(
        body.get("reasoning_effort").is_none(),
        "protocol layer should not inject reasoning_effort"
    );
}

#[test]
fn test_build_request_does_not_inject_reasoning_effort_high() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::High;
    let body = proto.build_request(&request).unwrap();
    assert!(
        body.get("reasoning_effort").is_none(),
        "protocol layer should not inject reasoning_effort"
    );
}

#[test]
fn test_build_request_does_not_inject_reasoning_effort_max() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Max;
    let body = proto.build_request(&request).unwrap();
    assert!(
        body.get("reasoning_effort").is_none(),
        "protocol layer should not inject reasoning_effort"
    );
}

#[test]
fn test_build_request_default_does_not_inject_reasoning_effort() {
    let proto = OpenAiProtocol::new();
    let request = make_request();
    let body = proto.build_request(&request).unwrap();
    assert!(
        body.get("reasoning_effort").is_none(),
        "protocol layer should not inject reasoning_effort by default"
    );
}

// ── Gap 1: non-streaming tool_calls parsing ─────────────────────────────────

#[test]
fn test_parse_response_with_tool_calls() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Let me check that.",
                "tool_calls": [{
                    "id": "call_001",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\": \"Beijing\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // Text block first, then ToolUse
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Let me check that.")
    );
    assert!(
        matches!(&resp.content_blocks[1], RawContentBlock::ToolUse { id, name, input }
            if id == "call_001" && name == "get_weather" && input == "{\"location\": \"Beijing\"}")
    );
}

#[test]
fn test_parse_response_with_tool_calls_only() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_002",
                    "type": "function",
                    "function": {
                        "name": "search",
                        "arguments": "{}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "total_tokens": 120
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // content=null + no reasoning → empty Text + ToolUse
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()));
    assert!(
        matches!(&resp.content_blocks[1], RawContentBlock::ToolUse { id, .. } if id == "call_002")
    );
}

#[test]
fn test_parse_response_with_multiple_tool_calls() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "call_a",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\": \"Shanghai\"}"
                        }
                    },
                    {
                        "id": "call_b",
                        "type": "function",
                        "function": {
                            "name": "get_time",
                            "arguments": "{\"timezone\": \"CST\"}"
                        }
                    },
                    {
                        "id": "call_c",
                        "type": "function",
                        "function": {
                            "name": "notify",
                            "arguments": "{}"
                        }
                    }
                ]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // empty Text + 3 ToolUse blocks
    assert_eq!(resp.content_blocks.len(), 4);
    assert!(matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()));
    assert!(
        matches!(&resp.content_blocks[1], RawContentBlock::ToolUse { id, name, .. }
        if id == "call_a" && name == "get_weather")
    );
    assert!(
        matches!(&resp.content_blocks[2], RawContentBlock::ToolUse { id, name, .. }
        if id == "call_b" && name == "get_time")
    );
    assert!(
        matches!(&resp.content_blocks[3], RawContentBlock::ToolUse { id, name, .. }
        if id == "call_c" && name == "notify")
    );
}

#[test]
fn test_parse_response_tool_calls_with_reasoning() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Here you go.",
                "reasoning_content": "Thinking about the request...",
                "tool_calls": [{
                    "id": "call_r1",
                    "type": "function",
                    "function": {
                        "name": "lookup",
                        "arguments": "{\"q\": \"rust\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // Text + Thinking + ToolUse (content + reasoning independent)
    assert_eq!(resp.content_blocks.len(), 3);
    assert!(matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Here you go."));
    assert!(
        matches!(&resp.content_blocks[1], RawContentBlock::Thinking { thinking, .. }
        if thinking == "Thinking about the request...")
    );
    assert!(
        matches!(&resp.content_blocks[2], RawContentBlock::ToolUse { id, name, .. }
        if id == "call_r1" && name == "lookup")
    );
}

// ── build_message tool result serialization ──────────────────────────────────

#[test]
fn test_build_message_tool_result() {
    let msg = super::InternalMessage {
        role: "tool".to_string(),
        content: r#"{"temperature": 22}"#.to_string(),
        tool_call_id: Some("call_abc".to_string()),
    };
    let value = super::build_message(&msg);
    assert_eq!(value["role"], "tool");
    assert_eq!(value["tool_call_id"], "call_abc");
    assert_eq!(value["content"], r#"{"temperature": 22}"#);
}

#[test]
fn test_build_message_tool_result_no_id_falls_back() {
    let msg = super::InternalMessage {
        role: "tool".to_string(),
        content: "result".to_string(),
        tool_call_id: None,
    };
    let value = super::build_message(&msg);
    assert_eq!(value["role"], "tool");
    assert!(value.get("tool_call_id").is_none());
}

#[test]
fn test_build_message_normal_user() {
    let msg = super::InternalMessage {
        role: "user".to_string(),
        content: "Hello".to_string(),
        tool_call_id: None,
    };
    let value = super::build_message(&msg);
    assert_eq!(value["role"], "user");
    assert_eq!(value["content"], "Hello");
    assert!(value.get("tool_call_id").is_none());
}

#[test]
fn test_build_request_includes_tool_result_message() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.messages.push(super::InternalMessage {
        role: "assistant".to_string(),
        content: String::new(),
        ..Default::default()
    });
    request.messages.push(super::InternalMessage {
        role: "tool".to_string(),
        content: r#"{"temp": 22}"#.to_string(),
        tool_call_id: Some("call_xyz".to_string()),
    });
    let body = proto.build_request(&request).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    let last = messages.last().unwrap();
    assert_eq!(last["role"], "tool");
    assert_eq!(last["tool_call_id"], "call_xyz");
}

#[test]
fn test_parse_response_cached_tokens() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150,
            "prompt_tokens_details": {
                "cached_tokens": 80
            }
        }
    });
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.usage.cache_read_tokens, Some(80));
    assert_eq!(resp.usage.cache_write_tokens, None);
}

#[test]
fn test_parse_response_no_cached_tokens() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.usage.cache_read_tokens, None);
    assert_eq!(resp.usage.cache_write_tokens, None);
}

// ── stream_options.include_usage injection ───────────────────────────────────

#[test]
fn test_build_request_stream_injects_stream_options() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.stream = true;
    let body = proto.build_request(&request).unwrap();
    let stream_options = body
        .get("stream_options")
        .expect("stream_options should exist");
    assert_eq!(stream_options["include_usage"], true);
}

#[test]
fn test_build_request_non_stream_does_not_inject_stream_options() {
    let proto = OpenAiProtocol::new();
    let request = make_request();
    // make_request() has stream=false by default
    let body = proto.build_request(&request).unwrap();
    assert!(
        body.get("stream_options").is_none(),
        "non-streaming request should not contain stream_options"
    );
}

// ── reasoning_tokens extraction from completion_tokens_details ─────────────

#[test]
fn test_parse_response_reasoning_tokens_extracted() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": { "role": "assistant", "content": "hi" },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 200,
            "total_tokens": 300,
            "completion_tokens_details": {
                "reasoning_tokens": 120
            }
        }
    });
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.usage.reasoning_tokens, Some(120));
    assert_eq!(resp.usage.completion_tokens, 200);
}

#[test]
fn test_parse_response_reasoning_tokens_missing() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": { "role": "assistant", "content": "hi" },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.usage.reasoning_tokens, None);
}

// ── SSE stream usage extraction ─────────────────────────────────────────────

/// Combined test for usage extraction: (1) usage in final chunk,
/// (2) no usage → None, (3) usage in same chunk as finish_reason.
#[tokio::test]
async fn test_sse_stream_usage_scenarios() {
    // Scenario 1: usage in final chunk alongside finish_reason
    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();
    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(r#"{"choices":[{"delta":{"content":"Hello"}}]}"#),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"content":" there!"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
        ),
    ]));
    let mut stream = proto.parse_sse_stream(incoming, machine).await;
    // BlockStart + 2 BlockDeltas + BlockEnd
    let _ = stream.next().await.unwrap().unwrap();
    let _ = stream.next().await.unwrap().unwrap();
    let _ = stream.next().await.unwrap().unwrap();
    let _ = stream.next().await.unwrap().unwrap();
    match stream.next().await.unwrap().unwrap() {
        StreamEvent::MessageEnd {
            usage,
            finish_reason,
        } => {
            let u = usage.unwrap();
            assert_eq!(u.prompt_tokens, 10);
            assert_eq!(u.completion_tokens, 5);
            assert_eq!(u.total_tokens, Some(15));
            assert_eq!(finish_reason.as_deref(), Some("stop"));
        }
        _ => panic!("expected MessageEnd"),
    }
    assert!(stream.next().await.is_none());

    // Scenario 2: no usage chunk → usage is None
    let proto2 = OpenAiProtocol::new();
    let m2 = proto2.create_sse_machine();
    let in2: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#),
        make_sse_chunk(r#"{"choices":[{"delta":{"content":"!"},"finish_reason":"stop"}]}"#),
    ]));
    let mut s2 = proto2.parse_sse_stream(in2, m2).await;
    for _ in 0..4 {
        let _ = s2.next().await.unwrap().unwrap();
    }
    match s2.next().await.unwrap().unwrap() {
        StreamEvent::MessageEnd { usage, .. } => {
            assert!(usage.is_none());
        }
        _ => panic!("expected MessageEnd"),
    }
    assert!(s2.next().await.is_none());

    // Scenario 3: usage in same chunk as finish_reason (no prior content)
    let proto3 = OpenAiProtocol::new();
    let m3 = proto3.create_sse_machine();
    let in3: IncomingSseStream = Box::pin(futures::stream::iter(vec![make_sse_chunk(
        r#"{"choices":[{"delta":{"content":"Done."},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}}"#,
    )]));
    let mut s3 = proto3.parse_sse_stream(in3, m3).await;
    let _ = s3.next().await.unwrap().unwrap(); // BlockStart
    let _ = s3.next().await.unwrap().unwrap(); // BlockDelta
    let _ = s3.next().await.unwrap().unwrap(); // BlockEnd
    match s3.next().await.unwrap().unwrap() {
        StreamEvent::MessageEnd { usage, .. } => {
            let u = usage.unwrap();
            assert_eq!(u.prompt_tokens, 20);
            assert_eq!(u.completion_tokens, 10);
            assert_eq!(u.total_tokens, Some(30));
        }
        _ => panic!("expected MessageEnd"),
    }
    assert!(s3.next().await.is_none());
}

#[tokio::test]
async fn test_sse_stream_tool_calls_with_usage() {
    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();
    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"call_x","type":"function","function":{"name":"search","arguments":""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{\"q\": \"rust\"}"}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":30,"completion_tokens":15,"total_tokens":45}}"#,
        ),
    ]));
    let mut stream = proto.parse_sse_stream(incoming, machine).await;
    for _ in 0..5 {
        let _ = stream.next().await.unwrap().unwrap();
    }
    match stream.next().await.unwrap().unwrap() {
        StreamEvent::MessageEnd {
            usage,
            finish_reason,
        } => {
            let u = usage.unwrap();
            assert_eq!(u.prompt_tokens, 30);
            assert_eq!(u.completion_tokens, 15);
            assert_eq!(u.total_tokens, Some(45));
            assert_eq!(finish_reason.as_deref(), Some("tool_calls"));
        }
        _ => panic!("expected MessageEnd"),
    }
    assert!(stream.next().await.is_none());
}

// ── Step 1.5: Provider raw JSON → Protocol parse_response integration ─────

/// Verify that the raw JSON format returned by `OpenAIProvider::send`
/// can be correctly parsed by `OpenAiProtocol::parse_response`.
/// This proves the Provider → Protocol handoff works end-to-end.
#[test]
fn test_parse_openai_provider_raw_json_response() {
    let proto = OpenAiProtocol::new();

    // Simulate the raw JSON that OpenAIProvider::send returns
    let raw_json = serde_json::json!({
        "id": "chatcmpl-test-123",
        "object": "chat.completion",
        "created": 1694268190,
        "model": "gpt-4",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello! How can I help you today?"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 8,
            "total_tokens": 18
        }
    });

    let resp = proto.parse_response(raw_json).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Hello! How can I help you today?")
    );
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 8);
    assert_eq!(resp.usage.total_tokens, Some(18));
    assert_eq!(resp.finish_reason, Some("stop".to_string()));
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.5: FETCH_MAX_ATTEMPTS constant usage verification
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that `FETCH_MAX_ATTEMPTS` is the correct constant name and value.
/// This verifies the rename from FETCH_MAX_RETRIES to FETCH_MAX_ATTEMPTS
/// and that the value remains 4 (4 total attempts = 3 retries).
#[test]
fn test_fetch_max_attempts_constant_value() {
    let val = crate::model_discovery::model_discovery_tests_only_fetch_max_attempts();
    assert_eq!(val, 4);
}

#[tokio::test]
async fn test_sse_stream_usage_only_in_dedicated_chunk() {
    // Usage arrives in a separate final chunk (no choices) before [DONE]
    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();
    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(r#"{"choices":[{"delta":{"content":"OK"},"finish_reason":"stop"}]}"#),
        make_sse_chunk(r#"{"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#),
    ]));
    let mut stream = proto.parse_sse_stream(incoming, machine).await;
    let _ = stream.next().await.unwrap().unwrap(); // BlockStart
    let _ = stream.next().await.unwrap().unwrap(); // BlockDelta
    let _ = stream.next().await.unwrap().unwrap(); // BlockEnd
    let mut found = false;
    while let Some(evt) = stream.next().await {
        if let StreamEvent::MessageEnd { usage, .. } = evt.unwrap() {
            let u = usage.unwrap();
            assert_eq!(u.prompt_tokens, 5);
            assert_eq!(u.completion_tokens, 2);
            assert_eq!(u.total_tokens, Some(7));
            found = true;
        }
    }
    assert!(found);
}

// ── Step 1.7: Protocol error detection tests ─────────────────────────────

/// Empty choices array → returns empty content blocks (graceful degradation).
#[test]
fn test_parse_response_empty_choices() {
    let proto = OpenAiProtocol::new();
    let body = serde_json::json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 0, "total_tokens": 10 } });
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()));
    assert!(resp.finish_reason.is_none());
}
