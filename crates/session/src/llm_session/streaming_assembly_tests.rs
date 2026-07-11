//! Unit tests for StreamingContentAssembler and SessionStream.

use super::streaming_assembly::{SessionStream, StreamingContentAssembler};
use closeclaw_common::processor::{
    ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedUsage,
};
use closeclaw_common::LLMError;
use futures::stream;
use futures::Stream;
use std::pin::Pin;

// ── StreamingContentAssembler tests ────────────────────────────────────────

#[test]
fn test_assembler_empty_stream() {
    let assembler = StreamingContentAssembler::new();
    assert!(assembler.content_blocks().is_empty());
    assert!(assembler.usage().is_none());
}

#[test]
fn test_assembler_text_block() {
    let mut assembler = StreamingContentAssembler::new();
    assembler.process_event(&StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::Text,
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Text {
            text: "hello".to_string(),
        },
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Text {
            text: " world".to_string(),
        },
    });
    assembler.process_event(&StreamEvent::BlockEnd {
        index: 0,
        block_type: ContentBlockType::Text,
    });

    let blocks = assembler.into_content_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0], ContentBlock::Text("hello world".to_string()));
}

#[test]
fn test_assembler_thinking_block() {
    let mut assembler = StreamingContentAssembler::new();
    assembler.process_event(&StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::Thinking,
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: "Let me think...".to_string(),
            signature: None,
        },
    });
    assembler.process_event(&StreamEvent::BlockEnd {
        index: 0,
        block_type: ContentBlockType::Thinking,
    });

    let blocks = assembler.into_content_blocks();
    assert_eq!(blocks.len(), 1);
    assert!(
        matches!(&blocks[0], ContentBlock::Thinking { thinking, .. } if thinking == "Let me think...")
    );
}

#[test]
fn test_assembler_tool_use_block() {
    let mut assembler = StreamingContentAssembler::new();
    assembler.process_event(&StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::ToolUse,
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ToolUseId {
            id: "call_1".to_string(),
        },
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ToolUseName {
            name: "bash".to_string(),
        },
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ToolUseInputChunk {
            input: "{\"cmd\":\"ls\"}".to_string(),
        },
    });
    assembler.process_event(&StreamEvent::BlockEnd {
        index: 0,
        block_type: ContentBlockType::ToolUse,
    });

    let blocks = assembler.into_content_blocks();
    assert_eq!(blocks.len(), 1);
    assert!(
        matches!(&blocks[0], ContentBlock::ToolUse { id, name, input }
            if id == "call_1" && name == "bash" && input == "{\"cmd\":\"ls\"}")
    );
}

#[test]
fn test_assembler_multiple_blocks() {
    let mut assembler = StreamingContentAssembler::new();

    // Thinking block at index 0
    assembler.process_event(&StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::Thinking,
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: "reasoning".to_string(),
            signature: None,
        },
    });
    assembler.process_event(&StreamEvent::BlockEnd {
        index: 0,
        block_type: ContentBlockType::Thinking,
    });

    // Text block at index 1
    assembler.process_event(&StreamEvent::BlockStart {
        index: 1,
        block_type: ContentBlockType::Text,
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 1,
        delta: ContentDelta::Text {
            text: "response".to_string(),
        },
    });
    assembler.process_event(&StreamEvent::BlockEnd {
        index: 1,
        block_type: ContentBlockType::Text,
    });

    let blocks = assembler.into_content_blocks();
    assert_eq!(blocks.len(), 2);
    assert!(matches!(&blocks[0], ContentBlock::Thinking { .. }));
    assert_eq!(blocks[1], ContentBlock::Text("response".to_string()));
}

#[test]
fn test_assembler_message_end_captures_usage() {
    let mut assembler = StreamingContentAssembler::new();
    let usage = UnifiedUsage {
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: Some(30),
        reasoning_tokens: Some(5),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    assembler.process_event(&StreamEvent::MessageEnd {
        usage: Some(usage.clone()),
        finish_reason: Some("stop".to_string()),
    });

    let captured = assembler.usage().unwrap();
    assert_eq!(captured.prompt_tokens, 10);
    assert_eq!(captured.completion_tokens, 20);
}

#[test]
fn test_assembler_error_preserves_partial() {
    let mut assembler = StreamingContentAssembler::new();
    assembler.process_event(&StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::Text,
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Text {
            text: "partial".to_string(),
        },
    });
    assembler.process_event(&StreamEvent::Error {
        message: "connection lost".to_string(),
    });

    let blocks = assembler.into_content_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0], ContentBlock::Text("partial".to_string()));
}

#[test]
fn test_assembler_delta_before_block_start_is_noop() {
    let mut assembler = StreamingContentAssembler::new();
    // Delta for index that was never started — should not panic.
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Text {
            text: "orphan".to_string(),
        },
    });
    assert!(assembler.content_blocks().is_empty());
}

#[test]
fn test_assembler_image_ref() {
    let mut assembler = StreamingContentAssembler::new();
    assembler.process_event(&StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::Image,
    });
    assembler.process_event(&StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ImageRef {
            name: "photo.jpg".to_string(),
            url: "https://example.com/photo.jpg".to_string(),
        },
    });
    assembler.process_event(&StreamEvent::BlockEnd {
        index: 0,
        block_type: ContentBlockType::Image,
    });

    let blocks = assembler.into_content_blocks();
    assert_eq!(blocks.len(), 1);
    assert!(
        matches!(&blocks[0], ContentBlock::Image { name, url } if name == "photo.jpg" && url == "https://example.com/photo.jpg")
    );
}

// ── SessionStream tests ────────────────────────────────────────────────────

fn make_text_stream(
    texts: Vec<&str>,
) -> Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>> {
    let mut events = Vec::new();
    for (i, text) in texts.iter().enumerate() {
        events.push(Ok(StreamEvent::BlockStart {
            index: i,
            block_type: ContentBlockType::Text,
        }));
        events.push(Ok(StreamEvent::BlockDelta {
            index: i,
            delta: ContentDelta::Text {
                text: text.to_string(),
            },
        }));
        events.push(Ok(StreamEvent::BlockEnd {
            index: i,
            block_type: ContentBlockType::Text,
        }));
    }
    events.push(Ok(StreamEvent::MessageEnd {
        usage: Some(default_usage()),
        finish_reason: Some("stop".to_string()),
    }));
    Box::pin(stream::iter(events))
}

fn default_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

#[tokio::test]
async fn test_session_stream_yields_events() {
    let inner = make_text_stream(vec!["hello", "world"]);
    let mut session_stream = SessionStream::new(inner);

    use futures::StreamExt;
    let mut collected = Vec::new();
    while let Some(event) = session_stream.next().await {
        collected.push(event.unwrap());
    }
    // 2 blocks × 3 events (start/delta/end) + MessageEnd = 7 events
    assert_eq!(collected.len(), 7);
    assert!(session_stream.is_finished());
}

#[tokio::test]
async fn test_session_stream_accumulates_blocks() {
    let inner = make_text_stream(vec!["hello", " world"]);
    let mut session_stream = SessionStream::new(inner);

    use futures::StreamExt;
    while let Some(item) = session_stream.next().await {
        let _ = item;
    }

    let blocks = session_stream.content_blocks();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0], ContentBlock::Text("hello".to_string()));
    assert_eq!(blocks[1], ContentBlock::Text(" world".to_string()));
}

#[tokio::test]
async fn test_session_stream_into_content_blocks() {
    let inner = make_text_stream(vec!["test"]);
    let mut session_stream = SessionStream::new(inner);

    use futures::StreamExt;
    while let Some(item) = session_stream.next().await {
        let _ = item;
    }

    let blocks = session_stream.into_content_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0], ContentBlock::Text("test".to_string()));
}

#[tokio::test]
async fn test_session_stream_error_preserves_partial() {
    let events = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "partial".to_string(),
            },
        }),
        Err(LLMError::ApiError("connection lost".to_string())),
    ];
    let inner: Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>> =
        Box::pin(stream::iter(events));
    let mut session_stream = SessionStream::new(inner);

    use futures::StreamExt;
    let mut results = Vec::new();
    while let Some(item) = session_stream.next().await {
        results.push(item);
    }

    // The error event should be propagated.
    assert!(results.last().unwrap().is_err());
    // Partial content should be preserved.
    let blocks = session_stream.content_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0], ContentBlock::Text("partial".to_string()));
}

#[tokio::test]
async fn test_session_stream_empty_stream() {
    let inner: Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>> =
        Box::pin(stream::iter(vec![]));
    let mut session_stream = SessionStream::new(inner);

    use futures::StreamExt;
    while let Some(item) = session_stream.next().await {
        let _ = item;
    }

    assert!(session_stream.content_blocks().is_empty());
    assert!(session_stream.is_finished());
}

#[tokio::test]
async fn test_session_stream_captures_usage() {
    let events = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "done".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let inner: Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>> =
        Box::pin(stream::iter(events));
    let mut session_stream = SessionStream::new(inner);

    use futures::StreamExt;
    while let Some(item) = session_stream.next().await {
        let _ = item;
    }

    let usage = session_stream.usage().unwrap();
    assert_eq!(usage.prompt_tokens, 5);
    assert_eq!(usage.completion_tokens, 10);
}
