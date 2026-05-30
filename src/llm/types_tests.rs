//! Tests for LLM types (serde, roundtrip, etc.).

use crate::llm::types::{
    ContentBlock, ContentBlockType, ContentDelta, SseStateMachine, StreamEvent, SystemBlock,
    UnifiedUsage,
};

// ── ContentBlock serde symmetry tests ──────────────────────────────────────

#[test]
fn test_content_block_text_serde_roundtrip() {
    let original = ContentBlock::Text("hello world".into());
    let json = serde_json::to_string(&original).unwrap();
    let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn test_content_block_thinking_serde_roundtrip() {
    let original = ContentBlock::Thinking("let me think...".into());
    let json = serde_json::to_string(&original).unwrap();
    let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn test_content_block_tool_use_serde_roundtrip() {
    let original = ContentBlock::ToolUse {
        id: "call_123".into(),
        name: "get_weather".into(),
        input: r#"{"city":"Beijing"}"#.into(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn test_content_block_tool_result_serde_roundtrip() {
    let original = ContentBlock::ToolResult {
        tool_call_id: "call_123".into(),
        content: "sunny, 25C".into(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(original, parsed);
}

// ── ContentBlock serde tag verification ───────────────────────────────────

#[test]
fn test_content_block_serialized_contains_type_field() {
    let blk = ContentBlock::Text("test".into());
    let json = serde_json::to_string(&blk).unwrap();
    assert!(
        json.contains(r#""type":"#),
        "JSON should contain \"type\" field: {json}"
    );

    let blk2 = ContentBlock::ToolUse {
        id: "x".into(),
        name: "y".into(),
        input: "{}".into(),
    };
    let json2 = serde_json::to_string(&blk2).unwrap();
    assert!(
        json2.contains(r#""type":"#),
        "JSON should contain \"type\" field: {json2}"
    );
}

// ── StreamEvent variant distinguishable test ───────────────────────────────

#[test]
fn test_stream_event_variants_are_distinguishable() {
    use StreamEvent::*;

    let events = [
        BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        },
        BlockDelta {
            index: 0,
            delta: ContentDelta::Text { text: "hi".into() },
        },
        BlockEnd {
            index: 0,
            block_type: ContentBlockType::Thinking,
        },
        MessageEnd {
            usage: None,
            finish_reason: Some("stop".into()),
        },
        Error {
            message: "oops".into(),
        },
    ];

    for e in &events {
        let json = serde_json::to_string(e).unwrap();
        let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, &parsed, "Roundtrip failed for {e:?}");
    }
}

// ── UnifiedUsage None serialization test ──────────────────────────────────

#[test]
fn test_unified_usage_optional_fields_none() {
    let usage = UnifiedUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: None,
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let json = serde_json::to_string(&usage).unwrap();
    let parsed: UnifiedUsage = serde_json::from_str(&json).unwrap();
    assert_eq!(usage, parsed);
    assert!(!json.contains("total_tokens"));
    assert!(!json.contains("reasoning_tokens"));
    assert!(!json.contains("cache_read_tokens"));
    assert!(!json.contains("cache_write_tokens"));
}

// ── SseStateMachine initial state test ────────────────────────────────────

#[test]
fn test_sse_state_machine_new_initial_state() {
    let sm = SseStateMachine::new();
    assert!(sm.current_block_index.is_none());
    assert!(sm.current_block_type.is_none());
    assert!(sm.pending_thinking.is_empty());
    assert!(sm.pending_signature.is_empty());
}

#[test]
fn test_sse_state_machine_default_same_as_new() {
    let sm = SseStateMachine::default();
    let sm2 = SseStateMachine::new();
    assert_eq!(sm.current_block_index, sm2.current_block_index);
    assert_eq!(sm.current_block_type, sm2.current_block_type);
    assert_eq!(sm.pending_thinking, sm2.pending_thinking);
    assert_eq!(sm.pending_signature, sm2.pending_signature);
}

// ── SystemBlock serde tests ───────────────────────────────────────────────

#[test]
fn test_system_block_serde_roundtrip() {
    let block = SystemBlock {
        text: "You are helpful.".to_string(),
        cache: true,
    };
    let json = serde_json::to_string(&block).unwrap();
    let parsed: SystemBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(block, parsed);
}

#[test]
fn test_system_block_cache_false_serde() {
    let block = SystemBlock {
        text: "dynamic".to_string(),
        cache: false,
    };
    let json = serde_json::to_string(&block).unwrap();
    let parsed: SystemBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(block, parsed);
    assert!(json.contains("cache"));
}

#[test]
fn test_unified_usage_cache_fields_some() {
    let usage = UnifiedUsage {
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: Some(150),
        reasoning_tokens: None,
        cache_read_tokens: Some(80),
        cache_write_tokens: Some(20),
    };
    let json = serde_json::to_string(&usage).unwrap();
    let parsed: UnifiedUsage = serde_json::from_str(&json).unwrap();
    assert_eq!(usage, parsed);
    assert!(json.contains("cache_read_tokens"));
    assert!(json.contains("cache_write_tokens"));
}
