// ------------------------------------------------------------------
// build_response_blocks unit tests
// ------------------------------------------------------------------

use super::*;
use crate::types::ProtocolKind;

#[test]
fn build_response_blocks_reasoning_shape() {
    let shape = ResponseShape::Reasoning(ReasoningResponse {
        content: "The answer is 42.".to_string(),
        reasoning: "Let me think step by step...".to_string(),
        signature: Some("sig-abc".to_string()),
        usage: None,
        ..Default::default()
    });
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);

    // Reasoning shape produces two blocks: reasoning + text
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block_type, "reasoning");
    // Medium intensity wraps the base reasoning
    assert!(
        blocks[0]
            .reasoning
            .as_deref()
            .unwrap()
            .contains("Let me think step by step..."),
        "reasoning should contain base text"
    );
    assert_eq!(blocks[0].signature.as_deref(), Some("sig-abc"));
    assert!(blocks[0].content.is_none());
    assert_eq!(blocks[1].block_type, "text");
    assert_eq!(blocks[1].content.as_deref(), Some("The answer is 42."));
}

#[test]
fn build_response_blocks_reasoning_without_signature() {
    let shape = ResponseShape::Reasoning(ReasoningResponse {
        content: "Result".to_string(),
        reasoning: "hmm".to_string(),
        signature: None,
        usage: None,
        ..Default::default()
    });
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block_type, "reasoning");
    assert!(blocks[0].signature.is_none());
    // Medium intensity wraps the base reasoning
    assert!(
        blocks[0].reasoning.as_deref().unwrap().contains("hmm"),
        "reasoning should contain base text"
    );
    assert_eq!(blocks[1].block_type, "text");
}

#[test]
fn build_response_blocks_tool_call_single() {
    let shape = ResponseShape::ToolCall(ToolCallResponse {
        calls: vec![ToolCallEntry {
            name: "get_weather".to_string(),
            arguments: r#"{"city":"Beijing"}"#.to_string(),
        }],
        usage: None,
    });
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, "tool_call");
    assert_eq!(blocks[0].tool_name.as_deref(), Some("get_weather"));
    assert_eq!(
        blocks[0].tool_arguments.as_deref(),
        Some(r#"{"city":"Beijing"}"#)
    );
    assert!(blocks[0].content.is_none());
    assert!(blocks[0].reasoning.is_none());
}

#[test]
fn build_response_blocks_tool_call_multiple() {
    let shape = ResponseShape::ToolCall(ToolCallResponse {
        calls: vec![
            ToolCallEntry {
                name: "search".to_string(),
                arguments: "{}".to_string(),
            },
            ToolCallEntry {
                name: "calc".to_string(),
                arguments: "{\"expr\": \"1+1\"}".to_string(),
            },
        ],
        usage: None,
    });
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);

    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block_type, "tool_call");
    assert_eq!(blocks[0].tool_name.as_deref(), Some("search"));
    assert_eq!(blocks[1].block_type, "tool_call");
    assert_eq!(blocks[1].tool_name.as_deref(), Some("calc"));
}

#[test]
fn build_response_blocks_text_shape() {
    let shape = ResponseShape::Text(TextResponse {
        content: "Hello world".to_string(),
        usage: None,
    });
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, "text");
    assert_eq!(blocks[0].content.as_deref(), Some("Hello world"));
}

#[test]
fn build_response_blocks_usage_shape_produces_no_blocks() {
    let shape = ResponseShape::Usage(UsageResponse::default());
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);
    // Usage-only shapes produce no response blocks — only usage data.
    assert!(blocks.is_empty());
}

#[test]
fn decide_reasoning_scenario_produces_correct_blocks() {
    let scenario = ScenarioDeclaration {
        name: "reasoning-scene".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Reasoning(ReasoningResponse {
                content: "42".to_string(),
                reasoning: "Let me think...".to_string(),
                signature: Some("sig1".to_string()),
                usage: None,
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let feat = features("gpt-4", "what is 6*7?");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "reasoning-scene");
            assert_eq!(d.response_blocks.len(), 2);
            assert_eq!(d.response_blocks[0].block_type, "reasoning");
            assert!(
                d.response_blocks[0]
                    .reasoning
                    .as_deref()
                    .unwrap()
                    .contains("Let me think..."),
                "reasoning should contain base text"
            );
            assert_eq!(d.response_blocks[1].block_type, "text");
            assert_eq!(d.response_blocks[1].content.as_deref(), Some("42"));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_tool_call_scenario_produces_correct_blocks() {
    let scenario = ScenarioDeclaration {
        name: "tool-call-scene".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::ToolCall(ToolCallResponse {
                calls: vec![ToolCallEntry {
                    name: "search".to_string(),
                    arguments: r#"{"q":"rust"}"#.to_string(),
                }],
                usage: None,
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let feat = features("gpt-4", "search for rust");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "tool-call-scene");
            assert_eq!(d.response_blocks.len(), 1);
            assert_eq!(d.response_blocks[0].block_type, "tool_call");
            assert_eq!(d.response_blocks[0].tool_name.as_deref(), Some("search"));
            assert_eq!(
                d.response_blocks[0].tool_arguments.as_deref(),
                Some(r#"{"q":"rust"}"#)
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_mixed_reasoning_and_tool_call_blocks() {
    let scenario2 = ScenarioDeclaration {
        name: "mixed-scene".to_string(),
        match_: None,
        turns: vec![
            TurnResponse {
                response: ResponseShape::Reasoning(ReasoningResponse {
                    content: String::new(),
                    reasoning: "Let me search...".to_string(),
                    signature: None,
                    usage: None,
                    ..Default::default()
                })
                .into(),
                delay: None,
                first_token_delay: None,
                segment_delay: None,
                stream_interrupt_after: None,
                error: None,
            },
            TurnResponse {
                response: ResponseShape::ToolCall(ToolCallResponse {
                    calls: vec![ToolCallEntry {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    }],
                    usage: None,
                })
                .into(),
                delay: None,
                first_token_delay: None,
                segment_delay: None,
                stream_interrupt_after: None,
                error: None,
            },
        ],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario2]).unwrap();
    let feat = features("gpt-4", "search for something");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            // Turn 0: reasoning response
            assert_eq!(d.response_blocks.len(), 2);
            assert_eq!(d.response_blocks[0].block_type, "reasoning");
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
    // Second request → turn 1: tool call
    let feat2 = RequestFeatures {
        model: "gpt-4".to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![
            MessageEntry {
                role: "user".to_string(),
                content: "search for something".to_string(),
            },
            MessageEntry {
                role: "assistant".to_string(),
                content: String::new(),
            },
            MessageEntry {
                role: "user".to_string(),
                content: "next".to_string(),
            },
        ],
        tools: vec![],
        protocol: ProtocolKind::OpenAi,
    };
    let outcome2 = engine.decide(&feat2);
    match outcome2 {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks.len(), 1);
            assert_eq!(d.response_blocks[0].block_type, "tool_call");
            assert_eq!(d.response_blocks[0].tool_name.as_deref(), Some("search"));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

// ------------------------------------------------------------------
// Composable Response Shapes (Step 1.3)
// ------------------------------------------------------------------

#[test]
fn build_response_blocks_composite_flattens() {
    let shape = ResponseShape::Composite(vec![
        ResponseShape::Text(TextResponse {
            content: "hello".to_string(),
            usage: None,
        }),
        ResponseShape::ToolCall(ToolCallResponse {
            calls: vec![ToolCallEntry {
                name: "search".to_string(),
                arguments: "{}".to_string(),
            }],
            usage: None,
        }),
    ]);
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block_type, "text");
    assert_eq!(blocks[0].content.as_deref(), Some("hello"));
    assert_eq!(blocks[1].block_type, "tool_call");
    assert_eq!(blocks[1].tool_name.as_deref(), Some("search"));
}

#[test]
fn build_response_blocks_nested_composite() {
    let shape = ResponseShape::Composite(vec![
        ResponseShape::Composite(vec![ResponseShape::Text(TextResponse {
            content: "a".to_string(),
            usage: None,
        })]),
        ResponseShape::Error(ErrorResponse {
            status: 500,
            message: "test error".to_string(),
            retry_after: None,
        }),
    ]);
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);
    // Error shapes produce no blocks (handled at engine level before block
    // construction). Only the nested text block is produced.
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, "text");
    assert_eq!(blocks[0].content.as_deref(), Some("a"));
}

#[test]
fn build_response_blocks_mixed_reasoning_and_tool_call() {
    let shape = ResponseShape::Composite(vec![
        ResponseShape::Reasoning(ReasoningResponse {
            content: "The answer.".to_string(),
            reasoning: "Let me think...".to_string(),
            signature: None,
            usage: None,
            ..Default::default()
        }),
        ResponseShape::ToolCall(ToolCallResponse {
            calls: vec![ToolCallEntry {
                name: "verify".to_string(),
                arguments: "{}".to_string(),
            }],
            usage: None,
        }),
    ]);
    let blocks = ScenarioEngine::build_response_blocks(&[shape]);
    // Reasoning produces 2 blocks (reasoning + text), tool_call produces 1
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].block_type, "reasoning");
    assert_eq!(blocks[1].block_type, "text");
    assert_eq!(blocks[2].block_type, "tool_call");
    assert_eq!(blocks[2].tool_name.as_deref(), Some("verify"));
}

#[test]
fn extract_usage_from_composite_shapes() {
    let text = ResponseShape::Text(TextResponse {
        content: "hello".to_string(),
        usage: None,
    });
    let usage_shape = ResponseShape::Usage(UsageResponse {
        prompt_tokens: Some(10),
        completion_tokens: Some(20),
        ..Default::default()
    });
    let shapes = vec![text, usage_shape];
    let usage = ScenarioEngine::extract_usage(&shapes);
    assert!(usage.is_some());
    let u = usage.unwrap();
    assert_eq!(u.prompt_tokens, Some(10));
    assert_eq!(u.completion_tokens, Some(20));
}

#[test]
fn extract_usage_first_shape_wins() {
    let text = ResponseShape::Text(TextResponse {
        content: "hello".to_string(),
        usage: Some(UsageResponse {
            prompt_tokens: Some(5),
            completion_tokens: Some(10),
            ..Default::default()
        }),
    });
    let usage_shape = ResponseShape::Usage(UsageResponse {
        prompt_tokens: Some(100),
        completion_tokens: Some(200),
        ..Default::default()
    });
    let shapes = vec![text, usage_shape];
    let usage = ScenarioEngine::extract_usage(&shapes);
    assert!(usage.is_some());
    let u = usage.unwrap();
    // First shape's usage wins
    assert_eq!(u.prompt_tokens, Some(5));
    assert_eq!(u.completion_tokens, Some(10));
}

#[test]
fn decide_composite_turn_produces_all_blocks() {
    let scenario = ScenarioDeclaration {
        name: "composite-scene".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseOrComposite::Multiple(vec![
                ResponseShape::Reasoning(ReasoningResponse {
                    content: "The result.".to_string(),
                    reasoning: "Thinking...".to_string(),
                    signature: None,
                    usage: None,
                    ..Default::default()
                }),
                ResponseShape::ToolCall(ToolCallResponse {
                    calls: vec![ToolCallEntry {
                        name: "run_code".to_string(),
                        arguments: r#"{"code": "1+1"}"#.to_string(),
                    }],
                    usage: None,
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let feat = features("gpt-4", "compute");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "composite-scene");
            // Reasoning produces 2 blocks, tool_call produces 1
            assert_eq!(d.response_blocks.len(), 3);
            assert_eq!(d.response_blocks[0].block_type, "reasoning");
            assert_eq!(d.response_blocks[1].block_type, "text");
            assert_eq!(d.response_blocks[2].block_type, "tool_call");
            assert_eq!(d.response_blocks[2].tool_name.as_deref(), Some("run_code"));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_composite_turn_with_usage() {
    let scenario = ScenarioDeclaration {
        name: "composite-usage".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseOrComposite::Multiple(vec![
                ResponseShape::Text(TextResponse {
                    content: "ok".to_string(),
                    usage: None,
                }),
                ResponseShape::Usage(UsageResponse {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(20),
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            // Text produces 1 block, Usage produces no blocks
            assert_eq!(d.response_blocks.len(), 1);
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("ok"));
            let u = d.usage.unwrap();
            assert_eq!(u.prompt_tokens, Some(10));
            assert_eq!(u.completion_tokens, Some(20));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}
