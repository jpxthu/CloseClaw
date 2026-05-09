//! Tests for OpenAI protocol — extracted to stay under 500-line limit.
use super::{
    ChatProtocol, ContentBlockType, ContentDelta, IncomingSseStream, InternalRequest,
    OpenAiProtocol, StreamEvent,
};
use crate::llm::types::RawSseChunk;
use futures::StreamExt;

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
