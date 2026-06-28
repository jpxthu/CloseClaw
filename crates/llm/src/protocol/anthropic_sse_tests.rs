//! SSE stream and build_message tests for Anthropic protocol.
//! Extracted from anthropic_tests.rs to stay under 1000-line limit.

use super::{
    AnthropicProtocol, ChatProtocol, ContentBlockType, ContentDelta, IncomingSseStream,
    InternalMessage, StreamEvent,
};
use crate::types::{InternalRequest, RawSseChunk};
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

// ── build_message tool result tests ──────────────────────────────────────
#[test]
fn test_build_message_tool_result() {
    let mut request = make_request();
    request.messages = vec![InternalMessage {
        role: "tool".to_string(),
        content: "25°C, sunny".to_string(),
        tool_call_id: Some("toolu_01A09q90qw90lq917835lq9".to_string()),
    }];
    let body = AnthropicProtocol::new().build_request(&request).unwrap();
    let msg = &body.get("messages").unwrap().as_array().unwrap()[0];
    assert_eq!(msg.get("role").unwrap(), "user");
    let content = msg.get("content").unwrap().as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0].get("type").unwrap(), "tool_result");
    assert_eq!(
        content[0].get("tool_use_id").unwrap(),
        "toolu_01A09q90qw90lq917835lq9"
    );
    assert_eq!(content[0].get("content").unwrap(), "25°C, sunny");
}
