//! CloseClaw-end streaming SSE parse_sse_stream contract tests.
//!
//! Loads every streaming fixture `.txt` (and companion `-meta.json`) from
//! `tests/fixtures/fake_llm/openai/` and `anthropic/`, feeds the raw SSE
//! text into `OpenAiProtocol::parse_sse_stream` /
//! `AnthropicProtocol::parse_sse_stream`, and asserts the resulting
//! `StreamEvent` sequence matches the contract defined in
//! `docs/design/llm/protocol-mapping.md`.

use super::fixture_loader::{
    anthropic_fixture_dir, load_streaming_fixture, load_streaming_meta, openai_fixture_dir,
};
use super::{AnthropicProtocol, ChatProtocol, OpenAiProtocol};
use crate::protocol::OutgoingEventStream;
use crate::types::{ContentBlockType, ContentDelta, RawSseChunk, StreamEvent};
use futures::StreamExt;

// ─── SSE text → RawSseChunk parser ──────────────────────────────────────────

/// A single parsed SSE event (event type + data JSON).
struct SseEvent {
    event_type: String,
    data: String,
}

/// Parse a raw SSE text block (as found in `.txt` fixture files) into
/// individual `SseEvent`s.
fn parse_sse_text(text: &str) -> Vec<SseEvent> {
    let mut events = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in text.lines() {
        if line.starts_with("event: ") {
            current_event = line[7..].to_string();
        } else if line.starts_with("data: ") {
            current_data = line[6..].to_string();
        } else if line.is_empty() && !current_data.is_empty() {
            events.push(SseEvent {
                event_type: std::mem::take(&mut current_event),
                data: std::mem::take(&mut current_data),
            });
            current_event.clear();
        }
    }
    // Flush last event if no trailing blank line
    if !current_data.is_empty() {
        events.push(SseEvent {
            event_type: std::mem::take(&mut current_event),
            data: std::mem::take(&mut current_data),
        });
    }
    events
}

/// Convert parsed `SseEvent`s into `RawSseChunk`s for the protocol parser.
fn to_raw_chunks(events: &[SseEvent]) -> Vec<RawSseChunk> {
    events
        .iter()
        .map(|e| RawSseChunk {
            event_type: e.event_type.clone(),
            data: e.data.clone(),
        })
        .collect()
}

// ─── Collect all events helper ──────────────────────────────────────────────

/// Consume all events from a stream and return them as a Vec.
async fn collect_events(stream: &mut OutgoingEventStream) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    while let Some(evt) = stream.next().await {
        events.push(evt.unwrap());
    }
    events
}

// ─── OpenAI streaming fixtures ──────────────────────────────────────────────

/// OpenAI text streaming: `streaming.txt`
///
/// Expected event sequence per protocol-mapping.md:
///   BlockStart(Text) → BlockDelta(Text, "Hello") → BlockDelta(Text, " there")
///   → BlockDelta(Text, " friend") → BlockDelta(Text, ".")
///   → BlockEnd(Text) → MessageEnd(stop)
#[tokio::test]
async fn openai_streaming_text_contract() {
    let raw = load_streaming_fixture(&openai_fixture_dir().join("streaming.txt")).unwrap();
    let meta = load_streaming_meta(&openai_fixture_dir().join("streaming-meta.json")).unwrap();

    // Verify meta provides request context
    assert_eq!(meta["request"]["stream_options"]["include_usage"], true);
    assert_eq!(meta["request"]["stream"], true);

    let events = parse_sse_text(&raw);
    let chunks = to_raw_chunks(&events);

    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();
    let incoming = Box::pin(futures::stream::iter(chunks));
    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let all = collect_events(&mut stream).await;

    // Expect: BlockStart + 4 BlockDelta + BlockEnd + MessageEnd = 7
    assert_eq!(
        all.len(),
        7,
        "expected 7 events, got {}: {:?}",
        all.len(),
        all
    );

    assert!(matches!(
        &all[0],
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }
    ));
    assert!(matches!(&all[1], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::Text { text }
    } if text == "Hello"));
    assert!(matches!(&all[2], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::Text { text }
    } if text == " there"));
    assert!(matches!(&all[3], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::Text { text }
    } if text == " friend"));
    assert!(matches!(&all[4], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::Text { text }
    } if text == "."));
    assert!(matches!(
        &all[5],
        StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }
    ));
    match &all[6] {
        StreamEvent::MessageEnd {
            usage,
            finish_reason,
        } => {
            assert_eq!(finish_reason.as_deref(), Some("stop"));
            assert!(
                usage.is_none(),
                "current impl emits usage=None on stream exhaustion"
            );
        }
        other => panic!("expected MessageEnd, got {:?}", other),
    }
}

/// OpenAI tool-use streaming: `tool-use-streaming.txt`
///
/// Expected event sequence per protocol-mapping.md:
///   BlockStart(ToolUse) → ToolUseId → ToolUseName → ToolUseInputChunk×N
///   → BlockEnd(ToolUse) → MessageEnd(tool_calls)
///
/// Assembled input_json_delta chunks should produce the same JSON as
/// the non-streaming `tool-use.json` fixture: `{"location":"Tokyo"}`.
#[tokio::test]
async fn openai_streaming_tool_use_contract() {
    let raw = load_streaming_fixture(&openai_fixture_dir().join("tool-use-streaming.txt")).unwrap();
    let meta =
        load_streaming_meta(&openai_fixture_dir().join("tool-use-streaming-meta.json")).unwrap();

    assert_eq!(meta["request"]["stream_options"]["include_usage"], true);
    assert!(meta["tools_sent"].is_array());

    let events = parse_sse_text(&raw);
    let chunks = to_raw_chunks(&events);

    let proto = OpenAiProtocol::new();
    let machine = proto.create_sse_machine();
    let incoming = Box::pin(futures::stream::iter(chunks));
    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let all = collect_events(&mut stream).await;

    // BlockStart(ToolUse) + ToolUseId + ToolUseName + 5×ToolUseInputChunk
    // + BlockEnd(ToolUse) + MessageEnd = 10
    assert!(
        all.len() >= 5,
        "should have at least 5 events, got {}: {:?}",
        all.len(),
        all
    );

    // 1. BlockStart(ToolUse, index=0)
    assert!(matches!(
        &all[0],
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
        }
    ));

    // 2. ToolUseId("call_fake_002")
    assert!(matches!(&all[1], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::ToolUseId { id }
    } if id == "call_fake_002"));

    // 3. ToolUseName("get_weather")
    assert!(matches!(&all[2], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::ToolUseName { name }
    } if name == "get_weather"));

    // Collect all input chunks
    let mut assembled_args = String::new();
    let mut input_chunk_count = 0;
    for evt in all.iter().skip(3) {
        match evt {
            StreamEvent::BlockDelta {
                delta: ContentDelta::ToolUseInputChunk { input },
                ..
            } => {
                assembled_args.push_str(input);
                input_chunk_count += 1;
            }
            StreamEvent::BlockEnd { .. } => break,
            _ => {}
        }
    }

    assert!(input_chunk_count > 0, "should have at least 1 input chunk");
    assert_eq!(
        assembled_args, r#"{"location":"Tokyo"}"#,
        "assembled args should match non-streaming fixture"
    );

    // Find BlockEnd(ToolUse) and MessageEnd
    let block_end = all.iter().position(|e| {
        matches!(
            e,
            StreamEvent::BlockEnd {
                block_type: ContentBlockType::ToolUse,
                ..
            }
        )
    });
    assert!(block_end.is_some(), "should have BlockEnd(ToolUse)");

    let msg_end = all
        .iter()
        .position(|e| matches!(e, StreamEvent::MessageEnd { .. }));
    assert!(msg_end.is_some(), "should have MessageEnd");

    if let Some(idx) = msg_end {
        if let StreamEvent::MessageEnd { finish_reason, .. } = &all[idx] {
            assert_eq!(finish_reason.as_deref(), Some("tool_calls"));
        }
    }
}

// ─── Anthropic streaming fixtures ───────────────────────────────────────────

/// Anthropic text streaming: `anthropic-streaming.txt`
///
/// Expected event sequence per protocol-mapping.md:
///   (message_start → usage captured)
///   BlockStart(Text) → BlockDelta(Text, "Hello") → BlockDelta(Text, " there")
///   → BlockDelta(Text, " friend") → BlockDelta(Text, ".")
///   → BlockEnd(Text) → MessageEnd(end_turn, usage)
///
/// Usage arrives in two phases: `message_start` gives input tokens,
/// `message_delta` gives output tokens + stop_reason.
/// `ping` events must be skipped.
#[tokio::test]
async fn anthropic_streaming_text_contract() {
    let raw =
        load_streaming_fixture(&anthropic_fixture_dir().join("anthropic-streaming.txt")).unwrap();
    let meta = load_streaming_meta(&anthropic_fixture_dir().join("anthropic-streaming-meta.json"))
        .unwrap();

    assert_eq!(meta["streaming"], true);
    assert_eq!(meta["protocol"], "anthropic");

    let events = parse_sse_text(&raw);
    let chunks = to_raw_chunks(&events);

    let proto = AnthropicProtocol::new();
    let machine = proto.create_sse_machine();
    let incoming = Box::pin(futures::stream::iter(chunks));
    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let all = collect_events(&mut stream).await;

    // BlockStart + 4 BlockDelta + BlockEnd + MessageEnd = 7
    assert_eq!(
        all.len(),
        7,
        "expected 7 events, got {}: {:?}",
        all.len(),
        all
    );

    assert!(matches!(
        &all[0],
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }
    ));
    assert!(matches!(&all[1], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::Text { text }
    } if text == "Hello"));
    assert!(matches!(&all[2], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::Text { text }
    } if text == " there"));
    assert!(matches!(&all[3], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::Text { text }
    } if text == " friend"));
    assert!(matches!(&all[4], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::Text { text }
    } if text == "."));
    assert!(matches!(
        &all[5],
        StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }
    ));
    match &all[6] {
        StreamEvent::MessageEnd {
            usage,
            finish_reason,
        } => {
            assert_eq!(finish_reason.as_deref(), Some("end_turn"));
            let u = usage.as_ref().expect("Anthropic should provide usage");
            assert_eq!(u.prompt_tokens, 11, "prompt_tokens from message_start");
            assert_eq!(
                u.completion_tokens, 4,
                "completion_tokens from message_delta"
            );
        }
        other => panic!("expected MessageEnd, got {:?}", other),
    }
}

/// Anthropic tool-use streaming: `anthropic-tool-use-streaming.txt`
///
/// Expected event sequence per protocol-mapping.md:
///   (message_start → input_tokens)
///   BlockStart(ToolUse) → ToolUseId → ToolUseName
///   → ToolUseInputChunk×N (partial_json segments)
///   → BlockEnd(ToolUse) → MessageEnd(tool_use, usage)
///
/// Assembled input_json_delta chunks should produce:
/// `{"location": "San Francisco"}` (matching non-streaming fixture).
#[tokio::test]
async fn anthropic_streaming_tool_use_contract() {
    let raw =
        load_streaming_fixture(&anthropic_fixture_dir().join("anthropic-tool-use-streaming.txt"))
            .unwrap();
    let meta = load_streaming_meta(
        &anthropic_fixture_dir().join("anthropic-tool-use-streaming-meta.json"),
    )
    .unwrap();

    assert_eq!(meta["protocol"], "anthropic");
    assert_eq!(meta["expect"], "tool_use");
    assert!(meta["tools_sent"].is_array());

    let events = parse_sse_text(&raw);
    let chunks = to_raw_chunks(&events);

    let proto = AnthropicProtocol::new();
    let machine = proto.create_sse_machine();
    let incoming = Box::pin(futures::stream::iter(chunks));
    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let all = collect_events(&mut stream).await;

    // BlockStart + ToolUseId + ToolUseName + 10×ToolUseInputChunk
    // + BlockEnd + MessageEnd = 14
    assert!(
        all.len() >= 5,
        "should have at least 5 events, got {}: {:?}",
        all.len(),
        all
    );

    // 1. BlockStart(ToolUse, index=0)
    assert!(matches!(
        &all[0],
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
        }
    ));

    // 2. ToolUseId
    assert!(matches!(
        &all[1],
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseId { .. },
        }
    ));

    // 3. ToolUseName("get_weather")
    assert!(matches!(&all[2], StreamEvent::BlockDelta {
        index: 0, delta: ContentDelta::ToolUseName { name }
    } if name == "get_weather"));

    // Collect all input chunks
    let mut assembled_args = String::new();
    let mut input_chunk_count = 0;
    for evt in all.iter().skip(3) {
        match evt {
            StreamEvent::BlockDelta {
                delta: ContentDelta::ToolUseInputChunk { input },
                ..
            } => {
                assembled_args.push_str(input);
                input_chunk_count += 1;
            }
            StreamEvent::BlockEnd { .. } => break,
            _ => {}
        }
    }

    assert!(input_chunk_count > 0, "should have at least 1 input chunk");
    assert_eq!(
        assembled_args, r#"{"location": "San Francisco"}"#,
        "assembled args should match non-streaming fixture"
    );

    // Find BlockEnd(ToolUse) and MessageEnd
    let block_end = all.iter().position(|e| {
        matches!(
            e,
            StreamEvent::BlockEnd {
                block_type: ContentBlockType::ToolUse,
                ..
            }
        )
    });
    assert!(block_end.is_some(), "should have BlockEnd(ToolUse)");

    let msg_end = all
        .iter()
        .position(|e| matches!(e, StreamEvent::MessageEnd { .. }));
    assert!(msg_end.is_some(), "should have MessageEnd");

    if let Some(idx) = msg_end {
        if let StreamEvent::MessageEnd {
            finish_reason,
            usage,
        } = &all[idx]
        {
            assert_eq!(finish_reason.as_deref(), Some("tool_use"));
            let u = usage.as_ref().expect("Anthropic should provide usage");
            assert_eq!(u.prompt_tokens, 39, "prompt_tokens from message_start");
            assert_eq!(
                u.completion_tokens, 45,
                "completion_tokens from message_delta"
            );
        }
    }
}

// ─── Coverage verification ──────────────────────────────────────────────────

/// Verify all streaming `.txt` fixtures across both protocols are
/// consumed by streaming contract tests (4 total).
#[test]
fn streaming_coverage_matrix() {
    let openai_dir = openai_fixture_dir();
    let anthropic_dir = anthropic_fixture_dir();

    let openai_txt: Vec<_> = std::fs::read_dir(&openai_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "txt").unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    let anthropic_txt: Vec<_> = std::fs::read_dir(&anthropic_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "txt").unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    // OpenAI: streaming.txt, tool-use-streaming.txt
    assert_eq!(
        openai_txt.len(),
        2,
        "expected 2 OpenAI streaming .txt fixtures, found {:?}",
        openai_txt
    );
    // Anthropic: anthropic-streaming.txt, anthropic-tool-use-streaming.txt
    assert_eq!(
        anthropic_txt.len(),
        2,
        "expected 2 Anthropic streaming .txt fixtures, found {:?}",
        anthropic_txt
    );
}
