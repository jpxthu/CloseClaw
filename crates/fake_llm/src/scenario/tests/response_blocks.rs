// ------------------------------------------------------------------
// build_response_blocks unit tests
// ------------------------------------------------------------------

use super::*;

#[test]
fn build_response_blocks_reasoning_shape() {
    let shape = ResponseShape::Reasoning(ReasoningResponse {
        content: "The answer is 42.".to_string(),
        reasoning: "Let me think step by step...".to_string(),
        signature: Some("sig-abc".to_string()),
        usage: None,
    });
    let blocks = ScenarioEngine::build_response_blocks(&shape);

    // Reasoning shape produces two blocks: reasoning + text
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block_type, "reasoning");
    assert_eq!(
        blocks[0].reasoning.as_deref(),
        Some("Let me think step by step...")
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
    });
    let blocks = ScenarioEngine::build_response_blocks(&shape);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block_type, "reasoning");
    assert!(blocks[0].signature.is_none());
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
    let blocks = ScenarioEngine::build_response_blocks(&shape);

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
    let blocks = ScenarioEngine::build_response_blocks(&shape);

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
    let blocks = ScenarioEngine::build_response_blocks(&shape);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, "text");
    assert_eq!(blocks[0].content.as_deref(), Some("Hello world"));
}

#[test]
fn build_response_blocks_usage_shape_produces_empty_text() {
    let shape = ResponseShape::Usage(UsageResponse::default());
    let blocks = ScenarioEngine::build_response_blocks(&shape);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, "text");
    assert_eq!(blocks[0].content.as_deref(), Some(""));
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
            }),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let feat = features("gpt-4", "what is 6*7?");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "reasoning-scene");
            assert_eq!(d.response_blocks.len(), 2);
            assert_eq!(d.response_blocks[0].block_type, "reasoning");
            assert_eq!(
                d.response_blocks[0].reasoning.as_deref(),
                Some("Let me think...")
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
            }),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
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
                }),
                delay: None,
                first_token_delay: None,
                segment_delay: None,
                error: None,
            },
            TurnResponse {
                response: ResponseShape::ToolCall(ToolCallResponse {
                    calls: vec![ToolCallEntry {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    }],
                    usage: None,
                }),
                delay: None,
                first_token_delay: None,
                segment_delay: None,
                error: None,
            },
        ],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario2]);
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
