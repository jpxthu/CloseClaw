//! Tests for OpenAI protocol — extracted to stay under 500-line limit.
use super::{
    ChatProtocol, ContentBlockType, ContentDelta, IncomingSseStream, InternalRequest,
    OpenAiProtocol, StreamEvent,
};
use crate::llm::types::{RawContentBlock, RawSseChunk};
use futures::StreamExt;

use crate::session::persistence::ReasoningLevel;

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
    // Empty content → only Thinking block (no empty Text)
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Thinking { thinking: s, .. } if s == "Let me think about this...")
    );
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
    // Both content and reasoning → only Text (content non-empty takes priority)
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "The answer is 42.")
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

// ── reasoning_level → reasoning_effort mapping tests ───────────────────────

#[test]
fn test_build_request_reasoning_level_low() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Low;
    let body = proto.build_request(&request).unwrap();
    assert_eq!(body.get("reasoning_effort").unwrap(), "off");
}

#[test]
fn test_build_request_reasoning_level_medium() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Medium;
    let body = proto.build_request(&request).unwrap();
    assert_eq!(body.get("reasoning_effort").unwrap(), "base");
}

#[test]
fn test_build_request_reasoning_level_high() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::High;
    let body = proto.build_request(&request).unwrap();
    assert_eq!(body.get("reasoning_effort").unwrap(), "high");
}

#[test]
fn test_build_request_reasoning_level_max() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.reasoning_level = ReasoningLevel::Max;
    let body = proto.build_request(&request).unwrap();
    assert_eq!(body.get("reasoning_effort").unwrap(), "reasoner");
}

#[test]
fn test_build_request_default_reasoning_level_is_high() {
    let proto = OpenAiProtocol::new();
    let request = make_request();
    let body = proto.build_request(&request).unwrap();
    assert_eq!(body.get("reasoning_effort").unwrap(), "high");
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
