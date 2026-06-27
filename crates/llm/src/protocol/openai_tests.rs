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
    // Both content and reasoning → Thinking + Text (thinking first)
    assert_eq!(resp.content_blocks.len(), 2);
    let RawContentBlock::Thinking { thinking, .. } = &resp.content_blocks[0] else {
        panic!("expected Thinking block");
    };
    assert_eq!(thinking, "Let me think about this...");
    assert!(
        matches!(&resp.content_blocks[1], RawContentBlock::Text(s) if s == "The answer is 42.")
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
    assert_eq!(resp.content_blocks.len(), 2);
    // Thinking block first
    match &resp.content_blocks[0] {
        RawContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "Let me think about this...");
            assert!(signature.is_none());
        }
        _ => panic!("Expected Thinking block first"),
    }
    // Text block second
    match &resp.content_blocks[1] {
        RawContentBlock::Text(text) => {
            assert_eq!(text, "The answer is 42.");
        }
        _ => panic!("Expected Text block second"),
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
    // Thinking + Text + ToolUse
    assert_eq!(resp.content_blocks.len(), 3);
    assert!(matches!(&resp.content_blocks[0],
        RawContentBlock::Thinking { thinking, .. } if thinking == "Thinking about the request..."));
    assert!(matches!(&resp.content_blocks[1], RawContentBlock::Text(s) if s == "Here you go."));
    assert!(
        matches!(&resp.content_blocks[2], RawContentBlock::ToolUse { id, name, .. }
        if id == "call_r1" && name == "lookup")
    );
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
