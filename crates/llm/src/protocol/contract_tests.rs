//! CloseClaw-end non-streaming parse_response contract tests.
//!
//! Loads every non-streaming protocol fixture from
//! `tests/fixtures/fake_llm/openai/` and `anthropic/`, feeds the
//! `response` JSON into `OpenAiProtocol::parse_response` /
//! `AnthropicProtocol::parse_response`, and asserts the resulting
//! `InternalResponse` against the contract defined in
//! `docs/design/llm/protocol-mapping.md`.

use super::fixture_loader::{anthropic_fixture_dir, load_protocol_fixture, openai_fixture_dir};
use super::{AnthropicProtocol, ChatProtocol, OpenAiProtocol};
use crate::types::{RawContentBlock, RawUsage};

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Extract a non-streaming, non-error fixture's `response` JSON value.
fn fixture_response(fixture_path: &std::path::Path) -> serde_json::Value {
    let fixture = load_protocol_fixture(fixture_path).unwrap();
    assert!(
        !fixture.streaming,
        "fixture {} should not be streaming",
        fixture_path.display()
    );
    fixture
        .response
        .unwrap_or_else(|| panic!("fixture {} has no response", fixture_path.display()))
}

/// Assert that a `RawUsage` field matches the expected value exactly.
fn assert_usage_eq(actual: &RawUsage, expected: &RawUsage) {
    assert_eq!(
        actual.prompt_tokens, expected.prompt_tokens,
        "prompt_tokens mismatch"
    );
    assert_eq!(
        actual.completion_tokens, expected.completion_tokens,
        "completion_tokens mismatch"
    );
    assert_eq!(
        actual.total_tokens, expected.total_tokens,
        "total_tokens mismatch"
    );
    assert_eq!(
        actual.cache_read_tokens, expected.cache_read_tokens,
        "cache_read_tokens mismatch"
    );
    assert_eq!(
        actual.cache_write_tokens, expected.cache_write_tokens,
        "cache_write_tokens mismatch"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// OpenAI Non-Streaming Contract Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn openai_simple_text() {
    let proto = OpenAiProtocol::new();
    let body = fixture_response(&openai_fixture_dir().join("simple.json"));
    let resp = proto.parse_response(body).unwrap();

    // content → Text block
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Hello there friend."),
        "expected single Text block, got {:?}",
        resp.content_blocks
    );

    // finish_reason
    assert_eq!(resp.finish_reason.as_deref(), Some("stop"));

    // usage: prompt_tokens_details.cached_tokens → cache_read_tokens
    assert_usage_eq(
        &resp.usage,
        &RawUsage {
            prompt_tokens: 14,
            completion_tokens: 4,
            total_tokens: Some(18),
            cache_read_tokens: Some(0),
            cache_write_tokens: None,
        },
    );
}

#[test]
fn openai_reasoning() {
    let proto = OpenAiProtocol::new();
    let body = fixture_response(&openai_fixture_dir().join("reasoning.json"));
    let resp = proto.parse_response(body).unwrap();

    // content + reasoning_content → Text + Thinking (both present)
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "391"),
        "expected Text block, got {:?}",
        resp.content_blocks[0]
    );
    assert!(
        matches!(
            &resp.content_blocks[1],
            RawContentBlock::Thinking { thinking, signature: None }
            if thinking == "The user asks for 17 * 23. Compute: 17 * 20 = 340, 17 * 3 = 51, sum = 391."
        ),
        "expected Thinking block, got {:?}",
        resp.content_blocks[1]
    );

    assert_eq!(resp.finish_reason.as_deref(), Some("stop"));

    assert_usage_eq(
        &resp.usage,
        &RawUsage {
            prompt_tokens: 14,
            completion_tokens: 24,
            total_tokens: Some(38),
            cache_read_tokens: Some(0),
            cache_write_tokens: None,
        },
    );
}

#[test]
fn openai_tool_use() {
    let proto = OpenAiProtocol::new();
    let body = fixture_response(&openai_fixture_dir().join("tool-use.json"));
    let resp = proto.parse_response(body).unwrap();

    // content="" + tool_calls → empty Text + ToolUse
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()),
        "expected empty Text block, got {:?}",
        resp.content_blocks[0]
    );
    assert!(
        matches!(
            &resp.content_blocks[1],
            RawContentBlock::ToolUse { id, name, input }
            if id == "call_fake_001"
               && name == "get_weather"
               && input == r#"{"location":"Tokyo"}"#
        ),
        "expected ToolUse block, got {:?}",
        resp.content_blocks[1]
    );

    assert_eq!(resp.finish_reason.as_deref(), Some("tool_calls"));

    assert_usage_eq(
        &resp.usage,
        &RawUsage {
            prompt_tokens: 18,
            completion_tokens: 12,
            total_tokens: Some(30),
            cache_read_tokens: Some(0),
            cache_write_tokens: None,
        },
    );
}

#[test]
fn openai_cache() {
    let proto = OpenAiProtocol::new();
    let body = fixture_response(&openai_fixture_dir().join("cache.json"));
    let resp = proto.parse_response(body).unwrap();

    // content → Text block
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.contains("HTTP keep-alive")),
        "expected Text block with keep-alive content, got {:?}",
        resp.content_blocks[0]
    );

    assert_eq!(resp.finish_reason.as_deref(), Some("stop"));

    // cache_read_tokens from prompt_tokens_details.cached_tokens
    assert_usage_eq(
        &resp.usage,
        &RawUsage {
            prompt_tokens: 38,
            completion_tokens: 72,
            total_tokens: Some(110),
            cache_read_tokens: Some(28),
            cache_write_tokens: None,
        },
    );
}

#[test]
fn openai_error_auth() {
    let proto = OpenAiProtocol::new();
    let fixture = load_protocol_fixture(&openai_fixture_dir().join("error-auth.json")).unwrap();
    let body = fixture.response.unwrap();

    // Error body has no "choices" → parse_response returns empty content blocks
    // (does not panic). With no content/reasoning/tool_calls the fallback is
    // a single empty Text block.
    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()),
        "expected single empty Text block for error response, got {:?}",
        resp.content_blocks
    );
}

#[test]
fn openai_error_rate_limit() {
    let proto = OpenAiProtocol::new();
    let fixture =
        load_protocol_fixture(&openai_fixture_dir().join("error-rate-limit.json")).unwrap();
    let body = fixture.response.unwrap();

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()),
        "expected single empty Text block for error response, got {:?}",
        resp.content_blocks
    );
}

#[test]
fn openai_error_server() {
    let proto = OpenAiProtocol::new();
    let fixture = load_protocol_fixture(&openai_fixture_dir().join("error-server.json")).unwrap();
    let body = fixture.response.unwrap();

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s.is_empty()),
        "expected single empty Text block for error response, got {:?}",
        resp.content_blocks
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Anthropic Non-Streaming Contract Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn anthropic_simple_text() {
    let proto = AnthropicProtocol::new();
    let body = fixture_response(&anthropic_fixture_dir().join("anthropic-simple.json"));
    let resp = proto.parse_response(body).unwrap();

    // text block → Text
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Hello there friend."),
        "expected single Text block, got {:?}",
        resp.content_blocks
    );

    // stop_reason → finish_reason
    assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));

    // usage: cache_read_input_tokens → cache_read_tokens,
    //        cache_creation_input_tokens → cache_write_tokens
    assert_usage_eq(
        &resp.usage,
        &RawUsage {
            prompt_tokens: 11,
            completion_tokens: 4,
            total_tokens: None,
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
        },
    );
}

#[test]
fn anthropic_thinking() {
    let proto = AnthropicProtocol::new();
    let body = fixture_response(&anthropic_fixture_dir().join("anthropic-thinking.json"));
    let resp = proto.parse_response(body).unwrap();

    // thinking block (with signature) + text block → Thinking + Text
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(
        matches!(
            &resp.content_blocks[0],
            RawContentBlock::Thinking { thinking, signature: Some(sig) }
            if thinking == "We need to compute 17 * 23. Using the distributive property: (10 + 7) * (20 + 3) = 10*20 + 10*3 + 7*20 + 7*3 = 200 + 30 + 140 + 21 = 391."
               && sig == "sig_thinking_b2c3d4e5f6a7b8c9"
        ),
        "expected Thinking block with signature, got {:?}",
        resp.content_blocks[0]
    );
    assert!(
        matches!(
            &resp.content_blocks[1],
            RawContentBlock::Text(s) if s.contains("To compute 17 * 23")
        ),
        "expected Text block, got {:?}",
        resp.content_blocks[1]
    );

    assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));

    assert_usage_eq(
        &resp.usage,
        &RawUsage {
            prompt_tokens: 18,
            completion_tokens: 90,
            total_tokens: None,
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
        },
    );
}

#[test]
fn anthropic_tool_use() {
    let proto = AnthropicProtocol::new();
    let body = fixture_response(&anthropic_fixture_dir().join("anthropic-tool-use.json"));
    let resp = proto.parse_response(body).unwrap();

    // thinking block + tool_use block → Thinking + ToolUse
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(
        matches!(
            &resp.content_blocks[0],
            RawContentBlock::Thinking { thinking, signature: Some(sig) }
            if thinking.contains("get_weather")
               && sig == "sig_tooluse_c3d4e5f6a7b8c9d0"
        ),
        "expected Thinking block with signature, got {:?}",
        resp.content_blocks[0]
    );
    assert!(
        matches!(
            &resp.content_blocks[1],
            RawContentBlock::ToolUse { id, name, input }
            if id == "toolu_fake_01_Vau98RhEyykRxCrGkYDe1551"
               && name == "get_weather"
               && input.contains("San Francisco")
        ),
        "expected ToolUse block, got {:?}",
        resp.content_blocks[1]
    );

    assert_eq!(resp.finish_reason.as_deref(), Some("tool_use"));

    assert_usage_eq(
        &resp.usage,
        &RawUsage {
            prompt_tokens: 39,
            completion_tokens: 45,
            total_tokens: None,
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
        },
    );
}

#[test]
fn anthropic_cache() {
    let proto = AnthropicProtocol::new();
    let body = fixture_response(&anthropic_fixture_dir().join("anthropic-cache.json"));
    let resp = proto.parse_response(body).unwrap();

    // thinking + text → Thinking + Text
    assert_eq!(resp.content_blocks.len(), 2);
    assert!(
        matches!(
            &resp.content_blocks[0],
            RawContentBlock::Thinking { thinking, signature: Some(sig) }
            if thinking.contains("HTTP/1.1")
               && sig == "sig_cache_d4e5f6a7b8c9d0e1"
        ),
        "expected Thinking block, got {:?}",
        resp.content_blocks[0]
    );
    assert!(
        matches!(
            &resp.content_blocks[1],
            RawContentBlock::Text(s) if s.contains("HTTP/1.1")
        ),
        "expected Text block, got {:?}",
        resp.content_blocks[1]
    );

    assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));

    // cache_read_input_tokens: 256 → cache_read_tokens
    // cache_creation_input_tokens: 0 → cache_write_tokens
    assert_usage_eq(
        &resp.usage,
        &RawUsage {
            prompt_tokens: 22,
            completion_tokens: 110,
            total_tokens: None,
            cache_read_tokens: Some(256),
            cache_write_tokens: Some(0),
        },
    );
}

#[test]
fn anthropic_error() {
    let proto = AnthropicProtocol::new();
    let fixture =
        load_protocol_fixture(&anthropic_fixture_dir().join("anthropic-error.json")).unwrap();
    let body = fixture.response.unwrap();

    // Error body: {"error": true, "body": {"type": "error", ...}}
    // parse_response_body expects "content" key → absent → empty blocks (no panic)
    let resp = proto.parse_response(body).unwrap();
    assert!(
        resp.content_blocks.is_empty(),
        "error response should have no content blocks, got {:?}",
        resp.content_blocks
    );
    // stop_reason absent in error body → finish_reason is None
    assert_eq!(resp.finish_reason, None);
}

// ═══════════════════════════════════════════════════════════════════════════
// Coverage matrix verification
// ═══════════════════════════════════════════════════════════════════════════

/// Verify all non-streaming OpenAI fixtures are consumed by contract tests.
#[test]
fn openai_non_streaming_coverage() {
    let entries = super::fixture_loader::load_protocol_fixtures_dir(&openai_fixture_dir()).unwrap();
    let non_streaming: Vec<_> = entries
        .iter()
        .filter(|e| !e.fixture.streaming && e.fixture.expect != "streaming")
        .collect();

    // simple, reasoning, tool-use, cache, error-auth, error-rate-limit,
    // error-server = 7 non-streaming fixtures
    assert_eq!(
        non_streaming.len(),
        7,
        "expected 7 non-streaming OpenAI fixtures, found {}: {:?}",
        non_streaming.len(),
        non_streaming.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
}

/// Verify all non-streaming Anthropic fixtures are consumed by contract tests.
#[test]
fn anthropic_non_streaming_coverage() {
    let entries =
        super::fixture_loader::load_protocol_fixtures_dir(&anthropic_fixture_dir()).unwrap();
    let non_streaming: Vec<_> = entries
        .iter()
        .filter(|e| !e.fixture.streaming && e.fixture.expect != "streaming")
        .collect();

    // anthropic-simple, anthropic-thinking, anthropic-tool-use,
    // anthropic-cache, anthropic-error = 5 non-streaming fixtures
    assert_eq!(
        non_streaming.len(),
        5,
        "expected 5 non-streaming Anthropic fixtures, found {}: {:?}",
        non_streaming.len(),
        non_streaming.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
}
