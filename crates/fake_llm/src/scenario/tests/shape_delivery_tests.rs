//! Step 1.3: Shape-level delivery control tests.
//!
//! Verifies that Streaming, Error, and Delay shapes carry configuration
//! payloads that the scenario engine extracts into decision delivery
//! fields, with correct priority rules and edge-case handling.

use crate::scenario::types::*;
use crate::types::{ProtocolKind, RequestFeatures};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn features(model: &str, msg: &str) -> RequestFeatures {
    RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![MessageEntry {
            role: "user".to_string(),
            content: msg.to_string(),
        }],
        tools: vec![],
        protocol: ProtocolKind::OpenAi,
    }
}

fn features_ext(model: &str, msgs: Vec<(&str, &str)>) -> RequestFeatures {
    RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: msgs
            .into_iter()
            .map(|(role, content)| MessageEntry {
                role: role.to_string(),
                content: content.to_string(),
            })
            .collect(),
        tools: vec![],
        protocol: ProtocolKind::OpenAi,
    }
}

fn text_turn(content: &str) -> TurnResponse {
    TurnResponse {
        response: ResponseShape::Text(TextResponse {
            content: content.to_string(),
            ..Default::default()
        })
        .into(),
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        error: None,
    }
}

/// Assert that the first response block contains the expected text.
fn assert_text_content(d: &super::ScenarioDecision, expected: &str) {
    assert_eq!(d.response_blocks[0].content.as_deref(), Some(expected));
}

/// Build a TurnResponse with an error shape and all other fields None.
fn make_error_turn(status: u16, message: &str, retry_after: Option<u64>) -> TurnResponse {
    TurnResponse {
        response: ResponseShape::Error(ErrorResponse {
            status,
            message: message.to_string(),
            retry_after,
        })
        .into(),
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        error: None,
    }
}

/// Assert delay fields on a `ScenarioDecision` with custom messages.
fn assert_delay_fields(
    d: &super::ScenarioDecision,
    delay: Option<u64>,
    first_token_delay: Option<u64>,
    segment_delay: Option<u64>,
) {
    assert_eq!(d.delay, delay, "delay mismatch");
    assert_eq!(
        d.first_token_delay, first_token_delay,
        "first_token_delay mismatch"
    );
    assert_eq!(d.segment_delay, segment_delay, "segment_delay mismatch");
}

// ===========================================================================
// Normal paths
// ===========================================================================

/// text+streaming combo: content blocks come from text, delivery params
/// come from streaming shape.
#[test]
fn streaming_text_combo_populates_delivery_params() {
    let scenario = ScenarioDeclaration {
        name: "combo".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseOrComposite::Multiple(vec![
                ResponseShape::Streaming(StreamingResponse {
                    segment_granularity: Some(5),
                    segment_delay_ms: Some(100),
                    ..Default::default()
                }),
                ResponseShape::Text(TextResponse {
                    content: "hello".to_string(),
                    ..Default::default()
                }),
            ]),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            // Content from text shape
            assert_eq!(d.response_blocks.len(), 1);
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("hello"));
            // Delivery params from streaming shape
            assert_eq!(d.segment_granularity, Some(5));
            assert_eq!(d.segment_delay, Some(100));
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Delay shape alone fills all three delay fields.
#[test]
fn delay_shape_alone_fills_all_delay_fields() {
    let scenario = ScenarioDeclaration {
        name: "delay-alone".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Delay(DelayResponse {
                delay_ms: Some(500),
                first_token_delay_ms: Some(200),
                segment_delay_ms: Some(50),
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            assert_eq!(d.delay, Some(500));
            assert_eq!(d.first_token_delay, Some(200));
            assert_eq!(d.segment_delay, Some(50));
            // Delay-only shape produces no content blocks
            assert!(d.response_blocks.is_empty());
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Error shape → DecisionOutcome::Error with correct fields.
#[test]
fn error_shape_returns_error_outcome() {
    let scenario = ScenarioDeclaration {
        name: "err-shape".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Error(ErrorResponse {
                status: 429,
                message: "rate limited".to_string(),
                retry_after: Some(30),
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 429);
            assert_eq!(e.message, "rate limited");
            assert_eq!(e.retry_after, Some(30));
        }
        super::DecisionOutcome::Decision(_) => panic!("expected error"),
    }
}

// ===========================================================================
// Priority: TurnResponse explicit > shape payload
// ===========================================================================

/// TurnResponse.delay=100 + Delay shape delay_ms=200 → decision delay=100.
#[test]
fn turn_response_delay_overrides_shape_delay() {
    let scenario = ScenarioDeclaration {
        name: "priority".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Delay(DelayResponse {
                delay_ms: Some(200),
                first_token_delay_ms: None,
                segment_delay_ms: None,
            })
            .into(),
            delay: Some(100),
            first_token_delay: None,
            segment_delay: None,
            error: None,
            stream_interrupt_after: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            assert_eq!(d.delay, Some(100), "TurnResponse explicit must win");
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// When TurnResponse has no delay fields, shape values fill them.
#[test]
fn shape_values_fill_when_turn_response_none() {
    let scenario = ScenarioDeclaration {
        name: "no-turn-delay".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Delay(DelayResponse {
                delay_ms: Some(300),
                first_token_delay_ms: Some(150),
                segment_delay_ms: Some(75),
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            error: None,
            stream_interrupt_after: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            assert_eq!(d.delay, Some(300));
            assert_eq!(d.first_token_delay, Some(150));
            assert_eq!(d.segment_delay, Some(75));
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

// ===========================================================================
// Edge cases
// ===========================================================================

/// Streaming empty object `{}` → deserializes and decision has all-None
/// delivery fields.
#[test]
fn streaming_empty_object_yields_none_fields() {
    let scenario = ScenarioDeclaration {
        name: "stream-empty".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Streaming(StreamingResponse::default()).into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            assert!(d.segment_granularity.is_none());
            assert!(d.segment_delay.is_none());
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Delay empty object `{}` → deserializes and decision has all-None fields.
#[test]
fn delay_empty_object_yields_none_fields() {
    let scenario = ScenarioDeclaration {
        name: "delay-empty".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Delay(DelayResponse::default()).into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            assert!(d.delay.is_none());
            assert!(d.first_token_delay.is_none());
            assert!(d.segment_delay.is_none());
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// segment_granularity=0 is preserved (means "single segment" in sse.rs).
#[test]
fn segment_granularity_zero_passthrough() {
    let scenario = ScenarioDeclaration {
        name: "gran-zero".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Streaming(StreamingResponse {
                segment_granularity: Some(0),
                ..Default::default()
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            assert_eq!(d.segment_granularity, Some(0));
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Multiple shapes declare the same delay field → first one wins.
#[test]
fn multiple_shapes_same_delay_first_wins() {
    let scenario = ScenarioDeclaration {
        name: "multi-delay".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseOrComposite::Multiple(vec![
                ResponseShape::Streaming(StreamingResponse {
                    segment_delay_ms: Some(100),
                    ..Default::default()
                }),
                ResponseShape::Delay(DelayResponse {
                    segment_delay_ms: Some(200),
                    delay_ms: Some(500),
                    first_token_delay_ms: None,
                }),
            ]),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            // Streaming shape is first → segment_delay=100 wins
            assert_eq!(d.segment_delay, Some(100));
            // Delay shape fills delay_ms (no TurnResponse override)
            assert_eq!(d.delay, Some(500));
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

// ===========================================================================
// State transitions: multi-turn isolation
// ===========================================================================

/// Turn 1 has delay shape, turn 2 has text → per-turn declarations are
/// independent and do not leak across turns.
#[test]
fn multi_turn_delay_isolation() {
    let scenario = ScenarioDeclaration {
        name: "turn-delay-iso".to_string(),
        match_: None,
        turns: vec![
            TurnResponse {
                response: ResponseShape::Delay(DelayResponse {
                    delay_ms: Some(500),
                    first_token_delay_ms: Some(100),
                    segment_delay_ms: Some(25),
                })
                .into(),
                delay: None,
                first_token_delay: None,
                segment_delay: None,
                stream_interrupt_after: None,
                error: None,
            },
            text_turn("hello from turn 2"),
        ],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();

    // Turn 0: delay shape fills all delay fields
    let feat0 = features("gpt-4", "hi");
    match engine.decide(&feat0) {
        super::DecisionOutcome::Decision(d) => {
            assert_delay_fields(&d, Some(500), Some(100), Some(25));
        }
        super::DecisionOutcome::Error(_) => {
            panic!("expected decision on turn 0")
        }
    }

    // Turn 1: no delay shape → all delay fields are None
    let feat1 = features_ext(
        "gpt-4",
        vec![("user", "hi"), ("assistant", "ok"), ("user", "next")],
    );
    match engine.decide(&feat1) {
        super::DecisionOutcome::Decision(d) => {
            assert_delay_fields(&d, None, None, None);
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("hello from turn 2")
            );
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision on turn 1"),
    }
}

/// Error shape in turn 2 → turn 1 is normal, turn 2 returns Error.
#[test]
fn multi_turn_error_in_second_turn() {
    let scenario = ScenarioDeclaration {
        name: "err-turn2".to_string(),
        match_: None,
        turns: vec![
            text_turn("first turn ok"),
            make_error_turn(503, "service unavailable", None),
        ],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();

    // Turn 0: normal text
    let feat0 = features("gpt-4", "hi");
    match engine.decide(&feat0) {
        super::DecisionOutcome::Decision(d) => assert_text_content(&d, "first turn ok"),
        super::DecisionOutcome::Error(_) => panic!("expected decision on turn 0"),
    }

    // Turn 1: error shape returns Error outcome
    let feat1 = features_ext(
        "gpt-4",
        vec![
            ("user", "hi"),
            ("assistant", "first turn ok"),
            ("user", "next"),
        ],
    );
    match engine.decide(&feat1) {
        super::DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 503);
            assert_eq!(e.message, "service unavailable");
        }
        super::DecisionOutcome::Decision(_) => panic!("expected error on turn 1"),
    }
}

// ===========================================================================
// Error paths
// ===========================================================================

/// TurnResponse.error and Error shape both present → TurnResponse.error wins.
#[test]
fn turn_response_error_overrides_shape_error() {
    let scenario = ScenarioDeclaration {
        name: "err-override".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Error(ErrorResponse {
                status: 503,
                message: "from shape".to_string(),
                retry_after: None,
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            error: Some(HttpError {
                status: 429,
                message: "from turn".to_string(),
                retry_after: None,
            }),
            stream_interrupt_after: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 429, "TurnResponse.error must take priority");
            assert_eq!(e.message, "from turn");
        }
        super::DecisionOutcome::Decision(_) => panic!("expected error"),
    }
}

/// Composite error+text → Error shape triggers early return, no content
/// blocks produced.
#[test]
fn composite_error_text_early_return_no_blocks() {
    let scenario = ScenarioDeclaration {
        name: "composite-err".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseOrComposite::Multiple(vec![
                ResponseShape::Error(ErrorResponse {
                    status: 401,
                    message: "unauthorized".to_string(),
                    retry_after: None,
                }),
                ResponseShape::Text(TextResponse {
                    content: "should not appear".to_string(),
                    ..Default::default()
                }),
            ]),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 401);
            assert_eq!(e.message, "unauthorized");
        }
        super::DecisionOutcome::Decision(d) => {
            // If we got a decision, no text blocks from the text shape
            // should be present (error caused early return)
            panic!(
                "expected error, got decision with {} blocks",
                d.response_blocks.len()
            );
        }
    }
}

// ===========================================================================
// Review B-2: Streaming shape usage extraction
// ===========================================================================

/// Streaming shape carries usage → decision usage is populated from it.
#[test]
fn streaming_shape_usage_populates_decision() {
    let scenario = ScenarioDeclaration {
        name: "streaming-usage".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Streaming(StreamingResponse {
                segment_granularity: Some(5),
                segment_delay_ms: None,
                usage: Some(UsageResponse {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(20),
                    ..Default::default()
                }),
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            let u = d.usage.expect("usage should be populated");
            assert_eq!(u.prompt_tokens, Some(10));
            assert_eq!(u.completion_tokens, Some(20));
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Streaming shape without usage does not interfere with Text shape usage.
#[test]
fn streaming_no_usage_does_not_affect_text_usage() {
    let scenario = ScenarioDeclaration {
        name: "streaming-no-usage".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseOrComposite::Multiple(vec![
                ResponseShape::Streaming(StreamingResponse {
                    segment_granularity: Some(3),
                    ..Default::default()
                }),
                ResponseShape::Text(TextResponse {
                    content: "hello".to_string(),
                    usage: Some(UsageResponse {
                        prompt_tokens: Some(5),
                        completion_tokens: Some(10),
                        ..Default::default()
                    }),
                }),
            ]),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            // Text shape usage should be used (Streaming has None)
            let u = d.usage.expect("usage should be populated");
            assert_eq!(u.prompt_tokens, Some(5));
            assert_eq!(u.completion_tokens, Some(10));
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Streaming shape usage takes priority when it appears before Text shape usage.
#[test]
fn streaming_usage_first_wins_over_text_usage() {
    let scenario = ScenarioDeclaration {
        name: "streaming-usage-first".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseOrComposite::Multiple(vec![
                ResponseShape::Streaming(StreamingResponse {
                    segment_granularity: Some(3),
                    segment_delay_ms: None,
                    usage: Some(UsageResponse {
                        prompt_tokens: Some(100),
                        completion_tokens: Some(200),
                        ..Default::default()
                    }),
                }),
                ResponseShape::Text(TextResponse {
                    content: "hello".to_string(),
                    usage: Some(UsageResponse {
                        prompt_tokens: Some(1),
                        completion_tokens: Some(2),
                        ..Default::default()
                    }),
                }),
            ]),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = super::ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&features("gpt-4", "hi"));
    match outcome {
        super::DecisionOutcome::Decision(d) => {
            // Streaming shape is first in the composite → its usage wins
            let u = d.usage.expect("usage should be populated");
            assert_eq!(u.prompt_tokens, Some(100));
            assert_eq!(u.completion_tokens, Some(200));
        }
        super::DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}
