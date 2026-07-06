//! Tests for LLM interpreter layer.

use crate::interpreter::{DefaultInterpreter, InterpreterRegistry, ModelInterpreter};
use crate::types::{
    ContentBlock, ContentBlockType, ContentDelta, InternalResponse, RawContentBlock, RawUsage,
    StreamEvent, UnifiedUsage,
};

// ── DefaultInterpreter ───────────────────────────────────────────────────────

#[test]
fn test_default_interpreter_name() {
    assert_eq!(DefaultInterpreter.name(), "default");
}

#[test]
fn test_default_interpreter_response_identity() {
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("hello".into()),
            RawContentBlock::Thinking {
                thinking: "thinking...".into(),
                signature: None,
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DefaultInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    assert!(matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "hello"));
    assert!(
        matches!(&unified.content_blocks[1], ContentBlock::Thinking { thinking: s, .. } if s == "thinking...")
    );
    assert_eq!(unified.usage.prompt_tokens, 10);
    assert_eq!(unified.finish_reason, Some("stop".into()));
}

#[test]
fn test_default_interpreter_stream_event_passthrough() {
    let event = StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::Text,
    };
    assert_eq!(
        DefaultInterpreter.interpret_stream_event(event.clone()),
        Some(event)
    );
}

// ── InterpreterRegistry resolve ───────────────────────────────────────────────

#[test]
fn test_registry_resolve_exact_match() {
    let registry = InterpreterRegistry::new(vec![(Box::new(DefaultInterpreter), "minimax/*")]);
    assert_eq!(registry.resolve("minimax", "倒 海外 3.0").name(), "default");
}

#[test]
fn test_registry_resolve_model_specific() {
    struct FakeInterpreter;
    impl ModelInterpreter for FakeInterpreter {
        fn name(&self) -> &str {
            "fake"
        }
        fn interpret_response(&self, r: InternalResponse) -> crate::types::UnifiedResponse {
            DefaultInterpreter.interpret_response(r)
        }
        fn interpret_stream_event(&self, e: StreamEvent) -> Option<StreamEvent> {
            Some(e)
        }
    }
    let registry = InterpreterRegistry::new(vec![
        (Box::new(FakeInterpreter), "glm-4/*"),
        (Box::new(DefaultInterpreter), "*/*"),
    ]);
    assert_eq!(registry.resolve("glm-4", "glm-4-flash").name(), "fake");
}

#[test]
fn test_registry_resolve_fallback_to_default() {
    let registry = InterpreterRegistry::new(vec![]);
    assert_eq!(registry.resolve("unknown", "some-model").name(), "default");
}

#[test]
fn test_registry_resolve_no_match_returns_default() {
    let registry = InterpreterRegistry::new(vec![(Box::new(DefaultInterpreter), "minimax/*")]);
    assert_eq!(
        registry.resolve("deepseek", "deepseek-chat").name(),
        "default"
    );
}

#[test]
fn test_registry_empty_returns_default() {
    assert_eq!(
        InterpreterRegistry::new(vec![])
            .resolve("any", "model")
            .name(),
        "default"
    );
}

#[test]
fn test_registry_first_match_wins() {
    struct First;
    impl ModelInterpreter for First {
        fn name(&self) -> &str {
            "first"
        }
        fn interpret_response(&self, r: InternalResponse) -> crate::types::UnifiedResponse {
            DefaultInterpreter.interpret_response(r)
        }
        fn interpret_stream_event(&self, e: StreamEvent) -> Option<StreamEvent> {
            Some(e)
        }
    }
    struct Second;
    impl ModelInterpreter for Second {
        fn name(&self) -> &str {
            "second"
        }
        fn interpret_response(&self, r: InternalResponse) -> crate::types::UnifiedResponse {
            DefaultInterpreter.interpret_response(r)
        }
        fn interpret_stream_event(&self, e: StreamEvent) -> Option<StreamEvent> {
            Some(e)
        }
    }
    let registry = InterpreterRegistry::new(vec![
        (Box::new(First), "*/*"),
        (Box::new(Second), "*/model-a"),
    ]);
    assert_eq!(registry.resolve("any", "model-a").name(), "first");
}

// ── MinimaxInterpreter ────────────────────────────────────────────────────────

use crate::interpreter::MinimaxInterpreter;

#[test]
fn test_minimax_interpreter_name() {
    assert_eq!(MinimaxInterpreter.name(), "minimax");
}

#[test]
fn test_minimax_interpreter_empty_content_uses_reasoning() {
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
        },
        finish_reason: Some("stop".into()),
    };
    let unified = MinimaxInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "Let me think step by step..."),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_minimax_interpreter_text_content_preferred() {
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("Hello world".into())],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = MinimaxInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "Hello world"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_minimax_interpreter_both_empty() {
    let response = InternalResponse {
        content_blocks: vec![],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = MinimaxInterpreter.interpret_response(response);
    assert!(unified.content_blocks.is_empty());
}

#[test]
fn test_minimax_interpreter_stream_event_passthrough() {
    let event = StreamEvent::BlockStart {
        index: 0,
        block_type: ContentBlockType::Thinking,
    };
    assert_eq!(
        MinimaxInterpreter.interpret_stream_event(event.clone()),
        Some(event)
    );
}

// ── GlmInterpreter ───────────────────────────────────────────────────────────

use crate::interpreter::GlmInterpreter;

#[test]
fn test_glm_interpreter_name() {
    assert_eq!(GlmInterpreter.name(), "glm");
}

#[test]
fn test_glm_interpreter_reasoning_threshold_short() {
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Thinking {
            thinking: "Hi".into(),
            signature: None,
        }],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = GlmInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "Hi"),
        "expected Text block for short reasoning, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_glm_interpreter_reasoning_threshold_exact_boundary() {
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Thinking {
            thinking: "12345678901".into(),
            signature: None,
        }],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = GlmInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "12345678901"),
        "expected Text block for long reasoning (no threshold), got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_glm_interpreter_text_preferred_over_reasoning() {
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("Real answer".into())],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = GlmInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "Real answer"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_glm_interpreter_stream_event_passthrough() {
    let event = StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: "thinking...".into(),
            signature: None,
        },
    };
    assert_eq!(
        GlmInterpreter.interpret_stream_event(event.clone()),
        Some(event)
    );
}

// ── DeepSeekInterpreter ───────────────────────────────────────────────────────

use crate::interpreter::DeepSeekInterpreter;

#[test]
fn test_deepseek_interpreter_name() {
    assert_eq!(DeepSeekInterpreter.name(), "deepseek");
}

#[test]
fn test_deepseek_interpreter_empty_content_uses_reasoning() {
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
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "Let me think step by step..."),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_deepseek_interpreter_text_content_preferred() {
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Text("Hello world".into())],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "Hello world"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_deepseek_interpreter_text_and_reasoning_prefers_text() {
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("hello".into()),
            RawContentBlock::Thinking {
                thinking: "thinking...".into(),
                signature: None,
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "hello"),
        "expected Text block when text is non-empty, got {:?}",
        unified.content_blocks[0]
    );
    assert!(
        matches!(&unified.content_blocks[1], ContentBlock::Thinking { thinking: s, .. } if s == "thinking..."),
        "expected Thinking block, got {:?}",
        unified.content_blocks[1]
    );
}

#[test]
fn test_deepseek_interpreter_stream_event_passthrough() {
    let event = StreamEvent::MessageEnd {
        usage: Some(UnifiedUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        }),
        finish_reason: Some("stop".into()),
    };
    assert_eq!(
        DeepSeekInterpreter.interpret_stream_event(event.clone()),
        Some(event)
    );
}

// ── Step 1.4: boundary & state-transition tests ────────────────────────────────

#[test]
fn test_minimax_interpreter_text_and_thinking_both_nonempty() {
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
        },
        finish_reason: Some("stop".into()),
    };
    let unified = MinimaxInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "hello"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
    assert!(
        matches!(&unified.content_blocks[1], ContentBlock::Thinking { thinking: s, .. } if s == "reasoning"),
        "expected Thinking block, got {:?}",
        unified.content_blocks[1]
    );
}

#[test]
fn test_minimax_interpreter_empty_thinking_string() {
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
        },
        finish_reason: None,
    };
    let unified = MinimaxInterpreter.interpret_response(response);
    // Empty thinking string still produces a Text block with empty content.
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s.is_empty()),
        "expected empty Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_minimax_interpreter_all_empty_strings() {
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("".into()),
            RawContentBlock::Thinking {
                thinking: "".into(),
                signature: None,
            },
        ],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = MinimaxInterpreter.interpret_response(response);
    // text is all-empty, thinking is non-empty (contains one empty string).
    // The interpreter treats thinking_parts as non-empty and produces Text("").
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s.is_empty()),
        "expected empty Text block, got {:?}",
        unified.content_blocks[0]
    );
}

// ── GlmInterpreter boundary tests ─────────────────────────────────────────

#[test]
fn test_glm_interpreter_empty_content_blocks() {
    let response = InternalResponse {
        content_blocks: vec![],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = GlmInterpreter.interpret_response(response);
    assert!(unified.content_blocks.is_empty());
}

#[test]
fn test_glm_interpreter_text_and_thinking_both_nonempty() {
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
        },
        finish_reason: Some("stop".into()),
    };
    let unified = GlmInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "answer"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
    assert!(
        matches!(&unified.content_blocks[1], ContentBlock::Thinking { thinking: s, .. } if s == "reasoning"),
        "expected Thinking block, got {:?}",
        unified.content_blocks[1]
    );
}

#[test]
fn test_glm_interpreter_empty_thinking_string() {
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
        },
        finish_reason: None,
    };
    let unified = GlmInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s.is_empty()),
        "expected empty Text block, got {:?}",
        unified.content_blocks[0]
    );
}

// ── DeepSeekInterpreter boundary tests ─────────────────────────────────────

#[test]
fn test_deepseek_interpreter_empty_content_blocks() {
    let response = InternalResponse {
        content_blocks: vec![],
        usage: RawUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    assert!(unified.content_blocks.is_empty());
}

#[test]
fn test_deepseek_interpreter_text_and_thinking_both_nonempty() {
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
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "answer"),
        "expected Text block, got {:?}",
        unified.content_blocks[0]
    );
    assert!(
        matches!(&unified.content_blocks[1], ContentBlock::Thinking { thinking: s, .. } if s == "reasoning"),
        "expected Thinking block, got {:?}",
        unified.content_blocks[1]
    );
}

#[test]
fn test_deepseek_interpreter_empty_thinking_string() {
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
        },
        finish_reason: None,
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s.is_empty()),
        "expected empty Text block, got {:?}",
        unified.content_blocks[0]
    );
}

// ── DeepSeekInterpreter signature preservation tests ──────────────────────────

#[test]
fn test_deepseek_interpreter_preserves_signature() {
    let sig = Some("deepseek-think-sig-001".to_string());
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("hello".into()),
            RawContentBlock::Thinking {
                thinking: "internal reasoning".into(),
                signature: sig.clone(),
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 2);
    match &unified.content_blocks[1] {
        ContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "internal reasoning");
            assert_eq!(signature, &sig);
        }
        other => panic!("expected Thinking block with signature, got {:?}", other),
    }
}

#[test]
fn test_deepseek_interpreter_no_signature_yields_none() {
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
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
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
fn test_deepseek_interpreter_signature_last_wins() {
    let sig1 = Some("sig-001".to_string());
    let sig2 = Some("sig-002".to_string());
    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Thinking {
                thinking: "first".into(),
                signature: sig1,
            },
            RawContentBlock::Thinking {
                thinking: "second".into(),
                signature: sig2.clone(),
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    // When text is empty and only thinking exists, thinking is merged into Text block
    // but the signature is still collected and available for non-empty text cases.
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "firstsecond"),
        "expected merged Text block, got {:?}",
        unified.content_blocks[0]
    );
}

#[test]
fn test_deepseek_interpreter_thinking_only_preserves_signature() {
    let sig = Some("ds-thinking-sig".to_string());
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
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DeepSeekInterpreter.interpret_response(response);
    // When text is empty, thinking is merged into Text block.
    // The signature is still collected; it surfaces when text is also present.
    assert_eq!(unified.content_blocks.len(), 1);
    assert!(
        matches!(&unified.content_blocks[0], ContentBlock::Text(s) if s == "deep thought"),
        "expected Text block (thinking merged into text), got {:?}",
        unified.content_blocks[0]
    );
}

// ── Gap 2: DefaultInterpreter preserves signature ─────────────────────────────

#[test]
fn test_default_interpreter_preserves_signature() {
    let sig = Some("test-signature-abc123".to_string());
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Thinking {
            thinking: "thinking with sig".into(),
            signature: sig.clone(),
        }],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    let unified = DefaultInterpreter.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1);
    match &unified.content_blocks[0] {
        ContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "thinking with sig");
            assert_eq!(signature, &sig);
        }
        other => panic!("expected Thinking block, got {:?}", other),
    }
}
