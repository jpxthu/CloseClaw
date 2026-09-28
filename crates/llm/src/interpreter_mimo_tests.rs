//! MimoInterpreter tests - extracted to stay under the 1000-line file limit.

use crate::interpreter::{MimoInterpreter, ModelInterpreter};
use crate::types::{ContentBlock, InternalResponse, RawContentBlock, RawUsage};

#[test]
fn test_mimo_interpreter_name() {
    assert_eq!(MimoInterpreter.name(), "mimo");
}

#[test]
fn test_mimo_interpreter_empty_text_uses_thinking() {
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
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(
            &unified.content_blocks[0],
            ContentBlock::Text(s) if s == "Let me think step by step..."
        ),
        "expected Text block (thinking merged), got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_mimo_interpreter_text_and_thinking_both_nonempty() {
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
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "hello"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
    assert!(
        matches!(
            &unified.content_blocks[1],
            ContentBlock::Thinking {
                thinking: s,
                ..
            } if s == "reasoning"
        ),
        "expected Thinking block, got {:?}",
        unified.content_blocks[1]
    );
}

#[test]
fn test_mimo_interpreter_text_only_no_thinking() {
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
    let unified = MimoInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "Hello world"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_mimo_interpreter_both_empty() {
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
    let unified = MimoInterpreter.interpret_response(response);
    assert!(unified.content_blocks.is_empty());
}
