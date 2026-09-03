//! Tests for AnthropicInterpreter — extracted from interpreter_test.rs
//! to stay under 1000-line limit.

use crate::interpreter::AnthropicInterpreter;
use crate::interpreter::ModelInterpreter;
use crate::types::{
    ContentBlock, ContentBlockType, ContentDelta, InternalResponse, RawContentBlock, RawUsage,
    StreamEvent,
};

#[test]
fn test_anthropic_interpreter_name() {
    assert_eq!(AnthropicInterpreter.name(), "anthropic");
}

#[test]
fn test_anthropic_interpreter_empty_text_uses_thinking() {
    // text empty + thinking non-empty → merged into Text block
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Thinking {
            thinking: "Let me think step by step...".into(),
            signature: None,
        }],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = AnthropicInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(
            &unified.content_blocks[0],
            ContentBlock::Text(s)
                if s == "Let me think step by step..."
        ),
        "expected Text block (thinking merged), got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_anthropic_interpreter_text_and_thinking_both_nonempty() {
    // text non-empty + thinking non-empty → Text + Thinking blocks
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("hello".into()),
            RawContentBlock::Thinking {
                thinking: "reasoning".into(),
                signature: None,
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = AnthropicInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "hello"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
    assert!(
        matches!(
            &unified.content_blocks[1],
            ContentBlock::Thinking { thinking: s, .. }
                if s == "reasoning"
        ),
        "expected Thinking block, got {:?}",
        unified.content_blocks[1]
    );
}

#[test]
fn test_anthropic_interpreter_text_only_no_thinking() {
    // text non-empty + thinking empty → only Text block
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("Hello world".into())],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: None,
    };
    let unified = AnthropicInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "Hello world"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_anthropic_interpreter_both_empty() {
    // both empty → no content blocks
    let response = InternalResponse {
        content_blocks: vec![],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: None,
    };
    let unified = AnthropicInterpreter.interpret_response(response);
    assert!(unified.content_blocks.is_empty());
}

#[test]
fn test_anthropic_interpreter_preserves_signature() {
    let sig = Some("test-signature-abc123".to_string());
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("hello".into()),
            RawContentBlock::Thinking {
                thinking: "thinking with sig".into(),
                signature: sig.clone(),
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = AnthropicInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    match &unified.content_blocks[1] {
        ContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "thinking with sig");
            assert_eq!(signature, &sig);
        }
        other => panic!("expected Thinking block with signature, got {:?}", other),
    }
}

#[test]
fn test_anthropic_interpreter_no_signature_yields_none() {
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("answer".into()),
            RawContentBlock::Thinking {
                thinking: "reasoning".into(),
                signature: None,
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = AnthropicInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    match &unified.content_blocks[1] {
        ContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "reasoning");
            assert_eq!(signature, &None);
        }
        other => panic!("expected Thinking block, got {:?}", other),
    }
}

#[test]
fn test_anthropic_interpreter_empty_text_signature_preserved_in_thinking() {
    // Empty text + thinking with signature → merged Text + separate Thinking with sig
    let sig = Some("sig-xyz".to_string());
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Thinking {
            thinking: "deep thought".into(),
            signature: sig.clone(),
        }],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = AnthropicInterpreter.interpret_response(response);
    // Empty text + non-empty thinking → merged into Text block
    // Signature preserved in a separate Thinking block
    assert_eq!(unified.content_blocks.len(), 2);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "deep thought"),
        "expected merged Text block, got {:?}",
        unified.content_blocks[0]
    );
    match &unified.content_blocks[1] {
        ContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert!(thinking.is_empty(), "expected empty thinking in sig block");
            assert_eq!(signature, &sig);
        }
        other => panic!("expected Thinking block with signature, got {:?}", other),
    }
}

#[test]
fn test_anthropic_interpreter_empty_thinking_string() {
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Thinking {
            thinking: "".into(),
            signature: None,
        }],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: None,
    };
    let unified = AnthropicInterpreter.interpret_response(response);
    // Empty text + thinking ("" counts as non-empty vec) → merged into Text block with empty string
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s.is_empty()),
        "expected empty Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_anthropic_interpreter_stream_event_passthrough() {
    let event = StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::Thinking,
    };
    assert_eq!(
        AnthropicInterpreter.interpret_stream_event(event.clone()),
        Some(event)
    );
}

#[test]
fn test_anthropic_interpreter_stream_signature_delta_passthrough() {
    let event = StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: String::new(),
            signature: Some("sig_abc".to_string()),
        },
    };
    assert_eq!(
        AnthropicInterpreter.interpret_stream_event(event.clone()),
        Some(event)
    );
}

#[test]
fn test_anthropic_interpreter_cache_usage_preserved() {
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("hi".into())],
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
    let unified = AnthropicInterpreter.interpret_response(response);
    assert_eq!(unified.usage.cache_read_tokens, Some(80));
    assert_eq!(unified.usage.cache_write_tokens, Some(20));
}
