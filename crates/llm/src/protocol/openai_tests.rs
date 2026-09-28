//! Tests for OpenAI protocol — extracted to stay under 500-line limit.
use super::{
    ChatProtocol, ContentBlockType, ContentDelta, IncomingSseStream, OpenAiProtocol, StreamEvent,
};
use crate::protocol::test_support::make_sse_chunk;
use crate::types::RawContentBlock;
use futures::StreamExt;

#[tokio::test]
async fn test_parse_sse_tool_calls_basic() {
    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(
            "{\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"call_abc\",\"type\":\"function\",\
            \"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]}}]}",
        ),
        make_sse_chunk(
            "{\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\
            \"arguments\":\"{\\\"location\\\"\"}}]}}]}",
        ),
        make_sse_chunk(
            "{\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\
            \"arguments\":\": \\\"Beijing\\\"}\"}}]}}]}",
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
        StreamEvent::BlockDelta {
            delta: ContentDelta::ToolUseName { name },
            ..
        } if name == "get_weather"
    ));

    // ToolUseInputChunk 1: {"location"
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            delta: ContentDelta::ToolUseInputChunk { input },
            ..
        } if input == r#"{"location""#
    ));

    // ToolUseInputChunk 2: : "Beijing"}
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            delta: ContentDelta::ToolUseInputChunk { input },
            ..
        } if input == ": \"Beijing\"}"
    ));

    // ToolUseInputChunk 3: }
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            delta: ContentDelta::ToolUseInputChunk { input },
            ..
        } if input == "}"
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
            "{\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"call_1\",\"type\":\"function\",\
            \"function\":{\"name\":\"search\",\"arguments\":\"\\\"query\\\"\"}}]}}]}",
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
        StreamEvent::BlockDelta {
            delta: ContentDelta::Text { text },
            ..
        } if text == " here's a tool call."
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
        StreamEvent::BlockDelta {
            delta: ContentDelta::ToolUseName { name },
            ..
        } if name == "search"
    ));

    // ToolUseInputChunk
    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt,
        StreamEvent::BlockDelta {
            delta: ContentDelta::ToolUseInputChunk { input },
            ..
        } if input == "\"query\""
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
            "{\"choices\":[{\"delta\":{\"content\":\" there!\"},\
            \"finish_reason\":\"stop\"}],\"usage\":{\
            \"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}",
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
        "{\"choices\":[{\"delta\":{\"content\":\"Done.\"},\"finish_reason\":\"stop\"}],\"usage\":{\
        \"prompt_tokens\":20,\"completion_tokens\":10,\"total_tokens\":30}}",
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
            "{\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"call_x\",\"type\":\"function\",\
            \"function\":{\"name\":\"search\",\"arguments\":\"\"}}]}}]}",
        ),
        make_sse_chunk(
            "{\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\
            \"arguments\":\"{\\\"q\\\": \\\"rust\\\"}\"}}]}}]}",
        ),
        make_sse_chunk(
            "{\"choices\":[{\"finish_reason\":\"tool_calls\"}],\"usage\":{\
            \"prompt_tokens\":30,\"completion_tokens\":15,\"total_tokens\":45}}",
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
    assert!(matches!(
        &resp.content_blocks[0],
        RawContentBlock::Text(s) if s == "Hello! How can I help you today?"
    ));
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 8);
    assert_eq!(resp.usage.total_tokens, Some(18));
    assert_eq!(resp.finish_reason, Some("stop".to_string()));
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
    let body = serde_json::json!({
        "choices": [],
        "usage": { "prompt_tokens": 10, "completion_tokens": 0, "total_tokens": 10 }
    });
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()));
    assert!(resp.finish_reason.is_none());
}

// ── Step 1.8: OpenAI business error body detection ─────────────────────

/// OpenAI error body (no choices array) → empty Text block (graceful).
#[test]
fn test_parse_response_openai_error_body() {
    let proto = OpenAiProtocol::new();
    let body =
        serde_json::json!({"error":{"message":"Invalid API key","type":"invalid_request_error"}});
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()));
    assert_eq!(resp.usage.prompt_tokens, 0);
    assert_eq!(resp.usage.completion_tokens, 0);
    assert!(resp.finish_reason.is_none());
}

// ── Step 1.6: content_blocks serialization tests ──────────────────────────
