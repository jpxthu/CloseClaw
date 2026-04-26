//! Unit tests for GLM streaming SSE parsing.

use crate::llm::glm_stream::{parse_sse_line, parse_stream_chunk, process_buffer};
use crate::llm::{ChatStreamChunk, Usage};

/// Collect all ChatStreamChunk items from a buffer processed through process_buffer.
fn collect_chunks(buffer: &[u8]) -> Vec<ChatStreamChunk> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    // Run process_buffer in a synchronous context (no async needed)
    process_buffer(buffer, &tx).expect("process_buffer should succeed");
    drop(tx); // close sender so rx closes

    let mut chunks = Vec::new();
    while let Some(chunk) = rx.blocking_recv() {
        chunks.push(chunk);
    }
    chunks
}

// ---------------------------------------------------------------------------
// Fixture: streaming-glm-4.7
// Expected: "1. **Analyze the Request**: The user has asked for a simple task: \"Count to 3\".\n2. **Determine..."
// Usage: prompt_tokens=9, completion_tokens=30, total_tokens=39, cached_tokens=2, reasoning_tokens=30
// ---------------------------------------------------------------------------

#[test]
fn test_glm_stream_parse_glm_4_7_fixture() {
    let fixture = include_str!("../../../tests/fixtures/llm/glm/streaming-glm-4.7.txt");

    // Find the first SSE stream in the fixture (starts at first "data: {")
    let start = fixture
        .find("data: {")
        .expect("fixture should contain data: {");
    let end = fixture
        .find("data: [DONE]")
        .expect("fixture should contain [DONE]");
    let stream = &fixture[start..end + "data: [DONE]".len()];

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    process_buffer(stream.as_bytes(), &tx).expect("process_buffer should succeed");
    drop(tx);

    let mut texts: Vec<String> = Vec::new();
    let mut done_model: Option<String> = None;
    let mut done_usage: Option<Usage> = None;

    while let Some(chunk) = rx.blocking_recv() {
        match chunk {
            ChatStreamChunk::Text(t) => texts.push(t),
            ChatStreamChunk::Done { model, usage } => {
                done_model = Some(model);
                done_usage = Some(usage);
            }
            ChatStreamChunk::Error(e) => panic!("unexpected error: {}", e),
        }
    }

    let full = texts.join("");
    assert!(
        full.starts_with("1.  **Analyze the Request:"),
        "text should start with expected prefix, got: {}",
        full
    );
    assert!(
        full.contains("Count to 3"),
        "text should contain 'Count to 3', got: {}",
        full
    );

    let usage = done_usage.expect("should have a Done chunk");
    assert_eq!(usage.prompt_tokens, 9);
    assert_eq!(usage.completion_tokens, 30);
    assert_eq!(usage.total_tokens, 39);

    assert_eq!(done_model.as_deref(), Some("glm-4.7"));
}

// ---------------------------------------------------------------------------
// Fixture: streaming-glm-5.1
// Usage: prompt_tokens=12, completion_tokens=50, total_tokens=62, cached_tokens=0, reasoning_tokens=50
// ---------------------------------------------------------------------------

#[test]
fn test_glm_stream_parse_glm_5_1_fixture() {
    let fixture = include_str!("../../../tests/fixtures/llm/glm/streaming-glm-5.1.txt");

    let start = fixture
        .find("data: {")
        .expect("fixture should contain data: {");
    let end = fixture
        .find("data: [DONE]")
        .expect("fixture should contain [DONE]");
    let stream = &fixture[start..end + "data: [DONE]".len()];

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    process_buffer(stream.as_bytes(), &tx).expect("process_buffer should succeed");
    drop(tx);

    let mut texts: Vec<String> = Vec::new();
    let mut done_usage: Option<Usage> = None;

    while let Some(chunk) = rx.blocking_recv() {
        match chunk {
            ChatStreamChunk::Text(t) => texts.push(t),
            ChatStreamChunk::Done { usage, .. } => done_usage = Some(usage),
            ChatStreamChunk::Error(e) => panic!("unexpected error: {}", e),
        }
    }

    let full = texts.join("");
    assert!(
        full.starts_with("1.  **Identify the core question:"),
        "text should start with expected prefix, got: {}",
        full
    );
    assert!(
        full.contains("2 and 2"),
        "text should contain '2 and 2', got: {}",
        full
    );

    let usage = done_usage.expect("should have a Done chunk");
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 50);
    assert_eq!(usage.total_tokens, 62);
}

// ---------------------------------------------------------------------------
// Fixture: streaming-glm-5.1-v2 (same usage format as 5.1)
// ---------------------------------------------------------------------------

#[test]
fn test_glm_stream_parse_glm_5_1_v2_fixture() {
    let fixture = include_str!("../../../tests/fixtures/llm/glm/streaming-glm-5.1-v2.txt");

    let start = fixture
        .find("data: {")
        .expect("fixture should contain data: {");
    let end = fixture
        .find("data: [DONE]")
        .expect("fixture should contain [DONE]");
    let stream = &fixture[start..end + "data: [DONE]".len()];

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    process_buffer(stream.as_bytes(), &tx).expect("process_buffer should succeed");
    drop(tx);

    let mut texts: Vec<String> = Vec::new();
    let mut done_usage: Option<Usage> = None;

    while let Some(chunk) = rx.blocking_recv() {
        match chunk {
            ChatStreamChunk::Text(t) => texts.push(t),
            ChatStreamChunk::Done { usage, .. } => done_usage = Some(usage),
            ChatStreamChunk::Error(e) => panic!("unexpected error: {}", e),
        }
    }

    let full = texts.join("");
    assert!(
        full.starts_with("1.  **Analyze the Input:"),
        "text should start with expected prefix, got: {}",
        full
    );
    assert!(
        full.contains("2+2"),
        "text should contain '2+2', got: {}",
        full
    );

    let usage = done_usage.expect("should have a Done chunk");
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 50);
    assert_eq!(usage.total_tokens, 62);
}

// ---------------------------------------------------------------------------
// parse_sse_line edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_parse_sse_line_empty_line() {
    assert_eq!(parse_sse_line(""), None);
    assert_eq!(parse_sse_line("   "), None);
    assert_eq!(parse_sse_line("\t"), None);
}

#[test]
fn test_parse_sse_line_non_data_line() {
    assert_eq!(parse_sse_line("event: message"), None);
    assert_eq!(parse_sse_line(":id: 123"), None);
    assert_eq!(parse_sse_line("comment: hello"), None);
}

#[test]
fn test_parse_sse_line_done() {
    assert_eq!(parse_sse_line("data: [DONE]"), Some("[DONE]"));
    assert_eq!(parse_sse_line("data:    [DONE]"), Some("[DONE]"));
}

#[test]
fn test_parse_sse_line_data_json() {
    let line = r#"data: {"id":"abc","choices":[]}"#;
    let parsed = parse_sse_line(line);
    assert!(parsed.is_some());
    let data = parsed.unwrap();
    assert!(data.starts_with('{'));
    assert!(data.contains("\"id\":\"abc\""));
}

#[test]
fn test_parse_sse_line_non_json_data_prefix() {
    // e.g. "data: hello world" — not JSON, should be skipped
    assert_eq!(parse_sse_line("data: hello world"), None);
    assert_eq!(parse_sse_line("data: 123"), None);
}

#[test]
fn test_parse_sse_line_whitespace_prefix() {
    let line = "  data: {\"id\":\"abc\"}";
    assert_eq!(parse_sse_line(line), Some("{\"id\":\"abc\"}"));
}

// ---------------------------------------------------------------------------
// parse_stream_chunk edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_parse_stream_chunk_delta_with_reasoning_content() {
    let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"thinking..."}}]}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let choice = chunk
        .choices
        .as_ref()
        .expect("should have choices")
        .first()
        .unwrap();
    let delta = choice.delta.as_ref().expect("should have delta");
    assert_eq!(delta.reasoning_content.as_deref(), Some("thinking..."));
    assert_eq!(delta.content.as_deref(), None);
}

#[test]
fn test_parse_stream_chunk_delta_with_content() {
    let json =
        r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","content":"hello"}}]}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let choice = chunk
        .choices
        .as_ref()
        .expect("should have choices")
        .first()
        .unwrap();
    let delta = choice.delta.as_ref().expect("should have delta");
    assert_eq!(delta.content.as_deref(), Some("hello"));
    assert_eq!(delta.reasoning_content.as_deref(), None);
}

#[test]
fn test_parse_stream_chunk_final_with_usage() {
    let json = r#"{"id":"abc","model":"glm-5.1","choices":[{"index":0,"finish_reason":"length","delta":{"role":"assistant","content":""}}],"usage":{"prompt_tokens":12,"completion_tokens":50,"total_tokens":62,"prompt_tokens_details":{"cached_tokens":0},"completion_tokens_details":{"reasoning_tokens":50}}}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let choice = chunk
        .choices
        .as_ref()
        .expect("should have choices")
        .first()
        .unwrap();
    assert_eq!(choice.finish_reason.as_deref(), Some("length"));

    let usage = chunk.usage.as_ref().expect("should have usage");
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 50);
    assert_eq!(usage.total_tokens, 62);
    assert_eq!(
        usage.prompt_tokens_details.as_ref().unwrap().cached_tokens,
        Some(0)
    );
    assert_eq!(
        usage
            .completion_tokens_details
            .as_ref()
            .unwrap()
            .reasoning_tokens,
        Some(50)
    );
}

#[test]
fn test_parse_stream_chunk_glm_error() {
    let json = r#"{"error":{"code":"1211","message":"模型不存在"}}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let err = chunk.error.as_ref().expect("should have error");
    assert_eq!(err.code, "1211");
    assert_eq!(err.message, "模型不存在");
}

#[test]
fn test_parse_stream_chunk_invalid_json() {
    let json = "not valid json";
    let result = parse_stream_chunk(json);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// process_chunk edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_process_chunk_no_choices() {
    let json = r#"{"id":"abc","choices":null}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let result = crate::llm::glm_stream::process_chunk(chunk, &tx);
    assert!(
        result.expect("should not error"),
        "should return true (continue)"
    );
    // No message sent
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_process_chunk_empty_delta() {
    let json = r#"{"id":"abc","model":"test","choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let result = crate::llm::glm_stream::process_chunk(chunk, &tx);
    assert!(
        result.expect("should not error"),
        "should return true (continue)"
    );
    // Empty delta → no Text chunk sent
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_process_chunk_multiple_choices() {
    let json = r#"{"id":"abc","model":"glm-5.1","choices":[
        {"index":0,"delta":{"role":"assistant","content":"first"}},
        {"index":1,"delta":{"role":"assistant","content":"second"}}
    ]}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    crate::llm::glm_stream::process_chunk(chunk, &tx).expect("should not error");

    let texts: Vec<String> = std::iter::from_fn(|| match rx.try_recv() {
        Ok(ChatStreamChunk::Text(t)) => Some(t),
        _ => None,
    })
    .collect();
    assert!(texts.contains(&"first".to_string()));
    assert!(texts.contains(&"second".to_string()));
}

#[test]
fn test_process_chunk_glm_error_maps_to_llm_error() {
    let json = r#"{"error":{"code":"1211","message":"模型不存在"}}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let err = crate::llm::glm_stream::process_chunk(chunk, &tx).unwrap_err();
    assert!(format!("{}", err).contains("模型不存在") || format!("{}", err).contains("1211"));
}

// ---------------------------------------------------------------------------
// merge_delta: reasoning_content preferred over content
// ---------------------------------------------------------------------------

#[test]
fn test_delta_content_preferred_over_reasoning() {
    let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","content":"visible","reasoning_content":"hidden"}}]}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    crate::llm::glm_stream::process_chunk(chunk, &tx).expect("should not error");
    match rx.blocking_recv() {
        Some(ChatStreamChunk::Text(t)) => assert_eq!(t, "visible"),
        other => panic!("expected Text, got {:?}", other),
    }
}

#[test]
fn test_delta_reasoning_content_when_content_empty() {
    let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning_content":"thinking..."}}]}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    crate::llm::glm_stream::process_chunk(chunk, &tx).expect("should not error");
    match rx.blocking_recv() {
        Some(ChatStreamChunk::Text(t)) => assert_eq!(t, "thinking..."),
        other => panic!("expected Text, got {:?}", other),
    }
}

#[test]
fn test_delta_empty_content_and_reasoning_sends_nothing() {
    let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning_content":""}}]}"#;
    let chunk = parse_stream_chunk(json).expect("should parse");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    crate::llm::glm_stream::process_chunk(chunk, &tx).expect("should not error");
    assert!(rx.try_recv().is_err(), "empty delta should send no chunk");
}

// ---------------------------------------------------------------------------
// process_buffer: integration of lines, partial data, done marker
// ---------------------------------------------------------------------------

#[test]
fn test_process_buffer_with_extra_sse_comments_and_blank_lines() {
    // SSE stream with blank lines and comments interspersed
    let data = "data: {\"id\":\"t1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"hi\"}}]}\n\n: this is a comment\n\ndata: {\"id\":\"t1\",\"choices\":[{\"index\":0,\"finish_reason\":\"stop\",\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2,\"total_tokens\":3}}\n\ndata: [DONE]\n";
    let chunks = collect_chunks(data.as_bytes());
    let texts: Vec<&str> = chunks
        .iter()
        .filter_map(|c| match c {
            ChatStreamChunk::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["hi"]);
    let done_chunk = chunks.last().expect("should have Done chunk");
    match done_chunk {
        ChatStreamChunk::Done { usage, .. } => {
            assert_eq!(usage.prompt_tokens, 1);
            assert_eq!(usage.completion_tokens, 2);
            assert_eq!(usage.total_tokens, 3);
        }
        other => panic!("expected Done, got {:?}", other),
    }
}

#[test]
fn test_process_buffer_multiple_sse_lines_in_one_read() {
    // Two delta chunks in a single buffer (simulating pipelined SSE delivery)
    let data = "data: {\"id\":\"t1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"hello\"}}]}\ndata: {\"id\":\"t1\",\"choices\":[{\"index\":0,\"finish_reason\":\"stop\",\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":5,\"total_tokens\":10}}\ndata: [DONE]\n";
    let chunks = collect_chunks(data.as_bytes());
    let texts: Vec<&str> = chunks
        .iter()
        .filter_map(|c| match c {
            ChatStreamChunk::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["hello"]);
    assert!(matches!(chunks.last(), Some(ChatStreamChunk::Done { .. })));
}

#[test]
fn test_process_buffer_done_marker_returns_consumed_bytes() {
    let data = "data: [DONE]\ndata: extra stuff\n";
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    // process_buffer should return Ok(consumed) when it hits [DONE]
    let result = process_buffer(data.as_bytes(), &tx);
    assert!(result.is_ok());
    // The consumed bytes should be up to and including "data: [DONE]\n"
    let consumed = result.unwrap();
    assert_eq!(consumed, "data: [DONE]\n".len());
}

#[test]
fn test_process_buffer_invalid_utf8_returns_error() {
    let data = b"data: \xff";
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let err = process_buffer(data, &tx).unwrap_err();
    assert!(format!("{}", err).contains("UTF-8") || format!("{}", err).contains("invalid"));
}
