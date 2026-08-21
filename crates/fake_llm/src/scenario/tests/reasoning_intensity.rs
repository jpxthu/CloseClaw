// ------------------------------------------------------------------
// Step 1.2: ReasoningIntensity tests
// ------------------------------------------------------------------

use super::*;

/// Helper: build a reasoning turn with given intensity and reasoning text.
fn reasoning_turn(content: &str, reasoning: &str, intensity: ReasoningIntensity) -> TurnResponse {
    TurnResponse {
        response: ResponseShape::Reasoning(ReasoningResponse {
            content: content.to_string(),
            reasoning: reasoning.to_string(),
            signature: None,
            usage: None,
            intensity,
        }),
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        error: None,
    }
}

/// Low intensity produces shorter reasoning text than Medium.
#[test]
fn reasoning_intensity_low_produces_shorter_text() {
    let scenario = ScenarioDeclaration {
        name: "intensity-low".to_string(),
        match_: None,
        turns: vec![reasoning_turn(
            "answer",
            "core thought",
            ReasoningIntensity::Low,
        )],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks.len(), 2);
            let reasoning_block = &d.response_blocks[0];
            assert_eq!(reasoning_block.block_type, "reasoning");
            let text = reasoning_block.reasoning.as_ref().unwrap();
            assert!(text.contains("core thought"));
            assert!(
                text.len() < 100,
                "Low intensity text too long: {}",
                text.len()
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Medium intensity produces moderate-length reasoning text.
#[test]
fn reasoning_intensity_medium_produces_moderate_text() {
    let scenario = ScenarioDeclaration {
        name: "intensity-medium".to_string(),
        match_: None,
        turns: vec![reasoning_turn(
            "answer",
            "core thought",
            ReasoningIntensity::Medium,
        )],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            let text = d.response_blocks[0].reasoning.as_ref().unwrap();
            assert!(text.contains("core thought"));
            assert!(
                text.len() > 50,
                "Medium intensity text too short: {}",
                text.len()
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// High intensity produces lengthy reasoning text.
#[test]
fn reasoning_intensity_high_produces_long_text() {
    let scenario = ScenarioDeclaration {
        name: "intensity-high".to_string(),
        match_: None,
        turns: vec![reasoning_turn(
            "answer",
            "core thought",
            ReasoningIntensity::High,
        )],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            let text = d.response_blocks[0].reasoning.as_ref().unwrap();
            assert!(text.contains("core thought"));
            assert!(
                text.len() >= 150,
                "High intensity text too short: {}",
                text.len()
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Three intensities produce strictly different text lengths:
/// Low < Medium < High.
#[test]
fn reasoning_intensity_ordering_low_lt_medium_lt_high() {
    fn intensity_len(intensity: ReasoningIntensity) -> usize {
        let scenario = ScenarioDeclaration {
            name: "len-test".to_string(),
            match_: None,
            turns: vec![reasoning_turn("a", "think about it", intensity)],
            models: None,
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let feat = features("gpt-4", "x");
        match engine.decide(&feat) {
            DecisionOutcome::Decision(d) => d.response_blocks[0].reasoning.as_ref().unwrap().len(),
            _ => panic!("expected decision"),
        }
    }

    let low_len = intensity_len(ReasoningIntensity::Low);
    let med_len = intensity_len(ReasoningIntensity::Medium);
    let high_len = intensity_len(ReasoningIntensity::High);

    assert!(
        low_len < med_len,
        "Low ({}) should be shorter than Medium ({})",
        low_len,
        med_len
    );
    assert!(
        med_len < high_len,
        "Medium ({}) should be shorter than High ({})",
        med_len,
        high_len
    );
}

/// Empty reasoning text stays empty regardless of intensity.
#[test]
fn reasoning_intensity_empty_reasoning_stays_empty() {
    let scenario = ScenarioDeclaration {
        name: "empty-reasoning".to_string(),
        match_: None,
        turns: vec![reasoning_turn("answer", "", ReasoningIntensity::High)],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            let text = d.response_blocks[0].reasoning.as_ref().unwrap();
            assert!(text.is_empty(), "empty reasoning should remain empty");
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Default intensity (Medium) is used when intensity field is absent.
#[test]
fn reasoning_intensity_default_medium_from_json() {
    let json = r#"{
        "response": {
            "type": "reasoning",
            "content": "ok",
            "reasoning": "think"
        }
    }"#;
    let turn: TurnResponse = serde_json::from_str(json).unwrap();
    match turn.response {
        ResponseShape::Reasoning(r) => {
            assert_eq!(r.intensity, ReasoningIntensity::Medium);
        }
        _ => panic!("expected Reasoning variant"),
    }
}
