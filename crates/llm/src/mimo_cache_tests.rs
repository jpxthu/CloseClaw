//! MiMo interpreter cache token passthrough tests.
//!
//! Split from `interpreter_test.rs` to keep file under 1000-line limit.
//! Tests the 5 behavioral dimensions for MiMo cache token transparency
//! per design doc `docs/design/llm/providers/mimo.md` §缓存机制.

use crate::interpreter::{MimoInterpreter, ModelInterpreter};
use crate::types::{ContentBlock, InternalResponse, RawContentBlock, RawUsage};

// ── Normal path: cache read tokens passthrough ─────────────────────────────

#[test]
fn test_mimo_interpreter_cache_read_tokens_passthrough() {
    // Normal path: cache hit tokens preserved
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("hello".into())],
        usage: RawUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: Some(150),
            cache_read_tokens: Some(42),
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(
        unified.usage.cache_read_tokens,
        Some(42),
        "cache_read_tokens should be preserved, not cleared"
    );
    assert_eq!(
        unified.usage.cache_write_tokens, None,
        "cache_write_tokens should remain None when not present"
    );
}

// ── Normal path: cache write tokens passthrough ────────────────────────────

#[test]
fn test_mimo_interpreter_cache_write_tokens_passthrough() {
    // If cache_write_tokens is present in raw response, it should also pass through
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("hello".into())],
        usage: RawUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: Some(150),
            cache_read_tokens: Some(80),
            cache_write_tokens: Some(20),
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(unified.usage.cache_read_tokens, Some(80));
    assert_eq!(unified.usage.cache_write_tokens, Some(20));
}

// ── Boundary: no cache field ───────────────────────────────────────────────

#[test]
fn test_mimo_interpreter_no_cache_field() {
    // Boundary: response without cache fields → None
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("hello".into())],
        usage: RawUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: Some(150),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(unified.usage.cache_read_tokens, None);
    assert_eq!(unified.usage.cache_write_tokens, None);
}

// ── Boundary: cached_tokens == 0 ───────────────────────────────────────────

#[test]
fn test_mimo_interpreter_cached_tokens_zero() {
    // Boundary: cached_tokens == 0 → Some(0), not None
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("hello".into())],
        usage: RawUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: Some(150),
            cache_read_tokens: Some(0),
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(
        unified.usage.cache_read_tokens,
        Some(0),
        "cached_tokens == 0 should be Some(0), not None"
    );
}

// ── Regression: thinking signature always None ─────────────────────────────

#[test]
fn test_mimo_interpreter_thinking_signature_always_none() {
    // Regression: thinking normalization — signature is always None
    let sig = Some("should-be-dropped".to_string());
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("answer".into()),
            RawContentBlock::Thinking {
                thinking: "reasoning".into(),
                signature: sig,
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: Some(3),
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    match &unified.content_blocks[1] {
        ContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "reasoning");
            assert_eq!(signature, &None, "MiMo signature must always be None");
        }
        other => panic!("expected Thinking block, got {:?}", other),
    }
    // Cache tokens should still pass through
    assert_eq!(unified.usage.cache_read_tokens, Some(3));
}

// ── Regression: empty thinking merge rule preserved ────────────────────────

#[test]
fn test_mimo_interpreter_empty_thinking_merge_preserved() {
    // Regression: empty text + non-empty thinking → merged into Text block
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Thinking {
            thinking: "step by step".into(),
            signature: None,
        }],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: Some(7),
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "step by step"),
        "expected Text block (thinking merged), got {:?}",
        unified.content_blocks[0]
    );
    assert_eq!(unified.usage.cache_read_tokens, Some(7));
}
