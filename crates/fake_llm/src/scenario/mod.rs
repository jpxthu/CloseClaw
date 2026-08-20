//! Scenario engine — core decision-making for Fake LLM Server.
//!
//! This module implements the scenario matching, session tracking, and
//! response generation pipeline. See `docs/design/fake_llm/scenario-engine.md`.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::types::{RequestFeatures, ScenarioDecision};

pub mod loader;
pub mod matcher;
pub mod session;
pub mod types;

pub use matcher::MatcherIndex;
pub use session::SessionTracker;
pub use types::*;

use loader::load_scenario_dir;

// ---------------------------------------------------------------------------
// Decision outcome
// ---------------------------------------------------------------------------

/// Outcome of a scenario engine decision.
///
/// Separates the two possible paths: a normal response decision or an
/// HTTP error injection. This avoids storing error fields inside
/// `ScenarioDecision` which is the happy-path type.
pub enum DecisionOutcome {
    /// Normal response — protocol layer serializes per protocol format.
    Decision(ScenarioDecision),
    /// HTTP error injection — endpoint returns this status code + message.
    Error(HttpError),
}

// ---------------------------------------------------------------------------
// ScenarioEngine
// ---------------------------------------------------------------------------

/// Core scenario engine: matches requests to scenarios and advances
/// multi-turn session cursors.
///
/// Held behind `Arc<Mutex<>>` in the server state, shared across all
/// request handlers.
pub struct ScenarioEngine {
    matcher: MatcherIndex,
    sessions: SessionTracker,
}

impl ScenarioEngine {
    /// Create a new engine by loading scenario files from the given directory.
    pub fn from_dir(dir: &std::path::Path) -> Result<Self> {
        let mut all_scenarios = Vec::new();
        let files = load_scenario_dir(dir)
            .with_context(|| format!("loading scenario dir: {}", dir.display()))?;
        for file in files {
            all_scenarios.extend(file.scenarios);
        }
        let matcher = MatcherIndex::build(all_scenarios);
        Ok(Self {
            matcher,
            sessions: SessionTracker::new(),
        })
    }

    /// Create an engine with an explicit scenario list (for testing).
    pub fn new(scenarios: Vec<ScenarioDeclaration>) -> Self {
        let matcher = MatcherIndex::build(scenarios);
        Self {
            matcher,
            sessions: SessionTracker::new(),
        }
    }

    /// Decide how to respond to the given request features.
    ///
    /// Flow: match scenario → advance session turn → build decision.
    pub fn decide(&mut self, features: &RequestFeatures) -> DecisionOutcome {
        let matched_idx = match self.matcher.match_request(features) {
            Some(idx) => idx,
            None => {
                // No scenario matched — return a placeholder response.
                return DecisionOutcome::Decision(ScenarioDecision {
                    model: features.model.clone(),
                    scenario: "default".to_string(),
                    stream: features.stream,
                    response_blocks: vec![ResponseBlock {
                        block_type: "text".to_string(),
                        content: Some("placeholder".to_string()),
                        tool_name: None,
                        tool_arguments: None,
                    }],
                    http_error: None,
                    delay: None,
                    usage: None,
                });
            }
        };

        let matched = self.matcher.get(matched_idx);

        // Extract message content strings for session tracking.
        let message_strings: Vec<String> = features
            .messages
            .iter()
            .map(|m| m.content.clone())
            .collect();

        let turn = self.sessions.advance_turn(&message_strings, &matched.name);

        if let Some(error) = &matched.turns[turn].error {
            return DecisionOutcome::Error(error.clone());
        }

        let turn_resp = &matched.turns[turn];
        let response_blocks = match &turn_resp.response {
            ResponseShape::Text(t) => vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some(t.content.clone()),
                tool_name: None,
                tool_arguments: None,
            }],
            ResponseShape::Usage(_) => vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some(String::new()),
                tool_name: None,
                tool_arguments: None,
            }],
            _ => vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("placeholder".to_string()),
                tool_name: None,
                tool_arguments: None,
            }],
        };

        let usage = match &turn_resp.response {
            ResponseShape::Usage(u) => Some(u.clone()),
            _ => None,
        };

        DecisionOutcome::Decision(ScenarioDecision {
            model: features.model.clone(),
            scenario: matched.name.clone(),
            stream: features.stream,
            response_blocks,
            http_error: None,
            delay: turn_resp.delay,
            usage,
        })
    }
}

/// Shared server state for Axum handlers.
#[derive(Clone)]
pub struct ScenarioState {
    pub engine: Arc<Mutex<ScenarioEngine>>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn text_turn(content: &str) -> TurnResponse {
        TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: content.to_string(),
            }),
            delay: None,
            error: None,
        }
    }

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
        }
    }

    #[test]
    fn decide_fallback_when_no_match() {
        let mut engine = ScenarioEngine::new(vec![]);
        let feat = features("gpt-4", "hello");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(d.scenario, "default");
                assert_eq!(d.response_blocks.len(), 1);
            }
            DecisionOutcome::Error(_) => panic!("expected decision, got error"),
        }
    }

    #[test]
    fn decide_matches_scenario_and_returns_turn() {
        let scenario = ScenarioDeclaration {
            name: "basic".to_string(),
            match_: None,
            turns: vec![text_turn("hello"), text_turn("world")],
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);

        let feat = features("gpt-4", "hi");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(d.scenario, "basic");
                assert_eq!(d.response_blocks[0].content.as_deref(), Some("hello"));
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }

        // Second request with extended history -> turn 1
        let feat2 = RequestFeatures {
            model: "gpt-4".to_string(),
            stream: false,
            max_tokens: None,
            temperature: None,
            messages: vec![
                MessageEntry {
                    role: "user".to_string(),
                    content: "hi".to_string(),
                },
                MessageEntry {
                    role: "assistant".to_string(),
                    content: "hello".to_string(),
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
                assert_eq!(d.response_blocks[0].content.as_deref(), Some("world"));
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    fn decide_error_injection() {
        let scenario = ScenarioDeclaration {
            name: "error-scene".to_string(),
            match_: None,
            turns: vec![TurnResponse {
                response: ResponseShape::Text(TextResponse {
                    content: String::new(),
                }),
                delay: None,
                error: Some(HttpError {
                    status: 500,
                    message: "server error".to_string(),
                }),
            }],
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let feat = features("gpt-4", "hi");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Error(e) => {
                assert_eq!(e.status, 500);
                assert_eq!(e.message, "server error");
            }
            DecisionOutcome::Decision(_) => panic!("expected error"),
        }
    }

    #[test]
    fn decide_captures_usage() {
        let scenario = ScenarioDeclaration {
            name: "usage-scene".to_string(),
            match_: None,
            turns: vec![TurnResponse {
                response: ResponseShape::Usage(UsageResponse {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(20),
                    reasoning_tokens: None,
                    cache_hit_tokens: None,
                    cache_write_tokens: None,
                }),
                delay: None,
                error: None,
            }],
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let feat = features("gpt-4", "hi");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                let u = d.usage.unwrap();
                assert_eq!(u.prompt_tokens, Some(10));
                assert_eq!(u.completion_tokens, Some(20));
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    fn decide_captures_delay() {
        let scenario = ScenarioDeclaration {
            name: "delay-scene".to_string(),
            match_: None,
            turns: vec![TurnResponse {
                response: ResponseShape::Text(TextResponse {
                    content: "slow".to_string(),
                }),
                delay: Some(500),
                error: None,
            }],
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let feat = features("gpt-4", "hi");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => assert_eq!(d.delay, Some(500)),
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }
}
