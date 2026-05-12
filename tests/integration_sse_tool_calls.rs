//! Integration tests for OpenAI SSE tool_calls parsing.
use closeclaw::llm::protocol::{ChatProtocol, IncomingSseStream, OpenAiProtocol};
use closeclaw::llm::types::{ContentBlockType, ContentDelta, RawSseChunk, StreamEvent};
use futures::StreamExt;

/// Helper to create SSE chunk matching test pattern
fn make_sse_chunk(data: &str) -> RawSseChunk {
    RawSseChunk {
        event_type: "message".to_string(),
        data: data.to_string(),
    }
}

/// Test basic single tool call SSE parsing
#[tokio::test]
async fn test_single_tool_call_basic() {
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
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":":\"Beijing\""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"}"}}]}}]}"#,
        ),
        make_sse_chunk(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::BlockStart {
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { delta: ContentDelta::ToolUseId { id }, .. } if id == "call_abc")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { delta: ContentDelta::ToolUseName { name }, .. } if name == "get_weather")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == r#"{"location""#)
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == ":\"Beijing\"")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == "}")
    );
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::BlockEnd {
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::MessageEnd { .. }
    ));
    assert!(stream.next().await.is_none());
}

/// Test arguments splitting across multiple chunks
#[tokio::test]
async fn test_tool_calls_arguments_chunking() {
    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"call_xyz","type":"function","function":{"name":"search","arguments":""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{"}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"\"query\""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":": "}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"\"rust\""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"}"}}]}}]}"#,
        ),
        make_sse_chunk(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::BlockStart {
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { delta: ContentDelta::ToolUseId { id }, .. } if id == "call_xyz")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { delta: ContentDelta::ToolUseName { name }, .. } if name == "search")
    );

    let mut chunks = Vec::new();
    loop {
        let evt = stream.next().await.unwrap().unwrap();
        match evt {
            StreamEvent::BlockDelta {
                delta: ContentDelta::ToolUseInputChunk { input },
                ..
            } => chunks.push(input),
            StreamEvent::BlockEnd {
                block_type: ContentBlockType::ToolUse,
                ..
            } => break,
            _ => panic!("Unexpected event in chunk collection"),
        }
    }

    assert_eq!(chunks.join(""), r#"{"query": "rust"}"#);
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::MessageEnd { .. }
    ));
    assert!(stream.next().await.is_none());
}

/// Test multiple tool calls behavior (matches current implementation)
/// Note: Current parser assigns all continuation chunks to the last active tool block
#[tokio::test]
async fn test_multiple_tool_calls() {
    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"call_2","type":"function","function":{"name":"get_time","arguments":""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{\"city\""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{\"tz\""}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{\"city\":\"Shanghai\"}"}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{\"tz\":\"UTC\"}"}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"}"}}]}}]}"#,
        ),
        make_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"}"}}]}}]}"#,
        ),
        make_sse_chunk(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    // Tool call 1 events (index 0)
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 0, delta: ContentDelta::ToolUseId { id }, .. } if id == "call_1")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 0, delta: ContentDelta::ToolUseName { name }, .. } if name == "get_weather")
    );

    // Tool call 2 events (index 1)
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 1, delta: ContentDelta::ToolUseId { id }, .. } if id == "call_2")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 1, delta: ContentDelta::ToolUseName { name }, .. } if name == "get_time")
    );

    // All continuation chunks assigned to last active block (index 1 - current implementation behavior)
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 1, delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == "{\"city\"")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 1, delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == "{\"tz\"")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 1, delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == r#"{"city":"Shanghai"}"#)
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 1, delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == r#"{"tz":"UTC"}"#)
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 1, delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == "}")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), StreamEvent::BlockDelta { index: 1, delta: ContentDelta::ToolUseInputChunk { input }, .. } if input == "}")
    );

    // Block end for last active block and message end
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::ToolUse,
            ..
        }
    ));
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::MessageEnd { .. }
    ));
    assert!(stream.next().await.is_none());
}
