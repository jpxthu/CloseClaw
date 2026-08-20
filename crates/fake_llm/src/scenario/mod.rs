//! Scenario engine — core decision-making for Fake LLM Server.
//!
//! This module implements the scenario matching, session tracking, and
//! response generation pipeline. See `docs/design/fake_llm/scenario-engine.md`.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::kv_cache::KvCacheSimulator;
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

/// Outcome of a models endpoint decision.
///
/// Three possible paths: a scenario-declared model list, an HTTP error,
/// or the default placeholder model list.
pub enum ModelsDecision {
    /// Scenario-declared model list.
    Models(Vec<ModelEntry>),
    /// HTTP error injection — endpoint returns this status code + message.
    Error(HttpError),
    /// No scenario declared models — use default placeholder list.
    Placeholder,
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
    kv_cache: KvCacheSimulator,
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
            kv_cache: KvCacheSimulator::new(),
        })
    }

    /// Create an engine with an explicit scenario list (for testing).
    pub fn new(scenarios: Vec<ScenarioDeclaration>) -> Self {
        let matcher = MatcherIndex::build(scenarios);
        Self {
            matcher,
            sessions: SessionTracker::new(),
            kv_cache: KvCacheSimulator::new(),
        }
    }

    /// Decide how to respond to the given request features.
    ///
    /// Flow: match scenario → advance session turn → build decision.
    pub fn decide(&mut self, features: &RequestFeatures) -> DecisionOutcome {
        let matched_idx = match self.matcher.match_request(features) {
            Some(idx) => idx,
            None => return Self::placeholder_decision(features),
        };

        let matched = self.matcher.get(matched_idx);
        let message_strings: Vec<String> = features
            .messages
            .iter()
            .map(|m| m.content.clone())
            .collect();
        let turn = self.sessions.advance_turn(&message_strings, &matched.name);

        let max_turns = matched.turns.len();
        if turn >= max_turns {
            panic!(
                "scenario '{}' exceeded declared turns (turn {}, max {})",
                matched.name, turn, max_turns
            );
        }

        let turn_resp = &matched.turns[turn];
        if let Some(error) = &turn_resp.error {
            return DecisionOutcome::Error(error.clone());
        }

        let response_blocks = Self::build_response_blocks(&turn_resp.response);
        let mut usage = Self::extract_usage(&turn_resp.response);
        let blocks = if response_blocks.is_empty() {
            vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("placeholder".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }]
        } else {
            response_blocks
        };

        // KV cache simulation: compute cache fields and merge into usage.
        // Explicit injection from the turn's usage takes priority over auto.
        let explicit_hit = usage.as_ref().and_then(|u| u.cache_hit_tokens);
        let explicit_write = usage.as_ref().and_then(|u| u.cache_write_tokens);
        let cache_result = self.kv_cache.process(
            &features.messages,
            &features.tools,
            explicit_hit,
            explicit_write,
        );
        Self::merge_cache_into_usage(&mut usage, &cache_result);

        DecisionOutcome::Decision(ScenarioDecision {
            model: features.model.clone(),
            scenario: matched.name.clone(),
            stream: features.stream,
            response_blocks: blocks,
            http_error: None,
            delay: turn_resp.delay,
            usage,
        })
    }

    /// Placeholder decision when no scenario matches.
    fn placeholder_decision(features: &RequestFeatures) -> DecisionOutcome {
        DecisionOutcome::Decision(ScenarioDecision {
            model: features.model.clone(),
            scenario: "default".to_string(),
            stream: features.stream,
            response_blocks: vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("placeholder".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            http_error: None,
            delay: None,
            usage: None,
        })
    }

    /// Decide how to respond to a `/v1/models` request.
    ///
    /// Finds the first matching scenario that declares a `models` list
    /// and returns it. If the current turn has an error injection, that
    /// takes priority. If no scenario declares models, returns `None`
    /// (caller should use the default model list).
    pub fn decide_for_models(&mut self) -> ModelsDecision {
        // Use a placeholder model for scenario matching since the
        // models endpoint has no request body with a model ID.
        let placeholder = RequestFeatures {
            model: String::new(),
            stream: false,
            max_tokens: None,
            temperature: None,
            messages: vec![],
            tools: vec![],
        };

        let matched_idx = match self.matcher.match_request(&placeholder) {
            Some(idx) => idx,
            None => return ModelsDecision::Placeholder,
        };

        let matched = self.matcher.get(matched_idx);
        let message_strings = vec![];
        let turn = self.sessions.advance_turn(&message_strings, &matched.name);

        let max_turns = matched.turns.len();
        if turn >= max_turns {
            panic!(
                "scenario '{}' exceeded declared turns (turn {}, max {})",
                matched.name, turn, max_turns
            );
        }

        let turn_resp = &matched.turns[turn];
        if let Some(error) = &turn_resp.error {
            return ModelsDecision::Error(error.clone());
        }

        if let Some(ref models) = matched.models {
            ModelsDecision::Models(models.clone())
        } else {
            ModelsDecision::Placeholder
        }
    }

    /// Build response blocks from a response shape.
    fn build_response_blocks(shape: &ResponseShape) -> Vec<ResponseBlock> {
        match shape {
            ResponseShape::Text(t) => vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some(t.content.clone()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            ResponseShape::Reasoning(r) => vec![
                ResponseBlock {
                    block_type: "reasoning".to_string(),
                    content: None,
                    tool_name: None,
                    tool_arguments: None,
                    reasoning: Some(r.reasoning.clone()),
                    signature: r.signature.clone(),
                },
                ResponseBlock {
                    block_type: "text".to_string(),
                    content: Some(r.content.clone()),
                    tool_name: None,
                    tool_arguments: None,
                    reasoning: None,
                    signature: None,
                },
            ],
            ResponseShape::ToolCall(tc) => tc
                .calls
                .iter()
                .map(|call| ResponseBlock {
                    block_type: "tool_call".to_string(),
                    content: None,
                    tool_name: Some(call.name.clone()),
                    tool_arguments: Some(call.arguments.clone()),
                    reasoning: None,
                    signature: None,
                })
                .collect(),
            ResponseShape::Usage(_) => vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some(String::new()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
            _ => vec![ResponseBlock {
                block_type: "text".to_string(),
                content: Some("placeholder".to_string()),
                tool_name: None,
                tool_arguments: None,
                reasoning: None,
                signature: None,
            }],
        }
    }

    /// Extract usage from a response shape, if present.
    fn extract_usage(shape: &ResponseShape) -> Option<UsageResponse> {
        match shape {
            ResponseShape::Usage(u) => Some(u.clone()),
            _ => None,
        }
    }

    /// Merge KV cache simulation results into usage fields.
    ///
    /// Only fills fields that are `None` (auto-simulated values).
    /// Explicit injection values already present in `usage` are preserved.
    fn merge_cache_into_usage(
        usage: &mut Option<UsageResponse>,
        cache: &crate::kv_cache::CacheResult,
    ) {
        if cache.cache_hit_tokens.is_none() && cache.cache_write_tokens.is_none() {
            return;
        }
        let u = usage.get_or_insert_with(UsageResponse::default);
        if u.cache_hit_tokens.is_none() {
            u.cache_hit_tokens = cache.cache_hit_tokens;
        }
        if u.cache_write_tokens.is_none() {
            u.cache_write_tokens = cache.cache_write_tokens;
        }
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
            models: None,
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
            models: None,
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
            models: None,
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
            models: None,
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let feat = features("gpt-4", "hi");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => assert_eq!(d.delay, Some(500)),
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    // ------------------------------------------------------------------
    // Fixture-loaded integration tests
    // ------------------------------------------------------------------

    /// Resolve the path to `tests/fixtures/fake_llm/scenarios/`.
    fn fixture_scenarios_dir() -> std::path::PathBuf {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .join("..")
            .join("..")
            .join("tests")
            .join("fixtures")
            .join("fake_llm")
            .join("scenarios")
    }

    fn features_with_model(model: &str, msg: &str) -> RequestFeatures {
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

    fn features_with_messages(model: &str, messages: Vec<(&str, &str)>) -> RequestFeatures {
        RequestFeatures {
            model: model.to_string(),
            stream: false,
            max_tokens: None,
            temperature: None,
            messages: messages
                .into_iter()
                .map(|(role, content)| MessageEntry {
                    role: role.to_string(),
                    content: content.to_string(),
                })
                .collect(),
            tools: vec![],
        }
    }

    #[test]
    fn decide_end_to_end_from_dir() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // basic-text.json: greeting scenario matches model "gpt-4o-basic" + "hello"
        let feat = features_with_model("gpt-4o-basic", "hello world");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(d.scenario, "greeting");
                assert_eq!(
                    d.response_blocks[0].content.as_deref(),
                    Some("Hi there! How can I help?")
                );
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    fn decide_fixture_fallback_basic() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // basic-text.json: fallback-basic matches model "gpt-4o-basic-fallback"
        let feat = features_with_model("gpt-4o-basic-fallback", "something else");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(d.scenario, "fallback-basic");
                assert_eq!(
                    d.response_blocks[0].content.as_deref(),
                    Some("I'm a fake LLM server.")
                );
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    fn decide_fixture_error_injection_rate_limit() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // error-injection.json: rate-limit scenario — first turn OK, second turn 429
        let feat = features_with_model("gpt-4o-error", "hi");
        let outcome1 = engine.decide(&feat);
        match outcome1 {
            DecisionOutcome::Decision(d) => {
                assert_eq!(d.scenario, "rate-limit");
                assert_eq!(
                    d.response_blocks[0].content.as_deref(),
                    Some("OK before error")
                );
            }
            DecisionOutcome::Error(_) => panic!("expected decision on first turn"),
        }

        // Second request: same session -> error injection
        let feat2 = RequestFeatures {
            model: "gpt-4o-error".to_string(),
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
                    content: "OK before error".to_string(),
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
            DecisionOutcome::Error(e) => {
                assert_eq!(e.status, 429);
                assert_eq!(e.message, "rate limit exceeded");
            }
            DecisionOutcome::Decision(_) => panic!("expected error on second turn"),
        }
    }

    #[test]
    fn decide_fixture_error_injection_server_error() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // error-injection.json: server-error matches model "gpt-4o-error-search" + tool "web_search"
        let feat = RequestFeatures {
            model: "gpt-4o-error-search".to_string(),
            stream: false,
            max_tokens: None,
            temperature: None,
            messages: vec![MessageEntry {
                role: "user".to_string(),
                content: "search something".to_string(),
            }],
            tools: vec!["web_search".to_string()],
        };
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Error(e) => {
                assert_eq!(e.status, 500);
                assert_eq!(e.message, "internal server error");
            }
            DecisionOutcome::Decision(_) => panic!("expected error"),
        }
    }

    #[test]
    fn decide_fixture_multi_turn_turn1() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // multi-turn.json: three-turn-chat with model "gpt-4o-multi"
        let feat = features_with_model("gpt-4o-multi", "start");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(d.scenario, "three-turn-chat");
                assert_eq!(
                    d.response_blocks[0].content.as_deref(),
                    Some("Turn 1: Hello!")
                );
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    fn decide_fixture_multi_turn_turn2() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // Drive to turn 1 first
        let feat1 = features_with_model("gpt-4o-multi", "start");
        let _ = engine.decide(&feat1);

        // Turn 2
        let feat = features_with_messages(
            "gpt-4o-multi",
            vec![
                ("user", "start"),
                ("assistant", "Turn 1: Hello!"),
                ("user", "continue"),
            ],
        );
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(
                    d.response_blocks[0].content.as_deref(),
                    Some("Turn 2: How are you?")
                );
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    fn decide_fixture_multi_turn_turn3() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // Drive to turn 2
        let feat1 = features_with_model("gpt-4o-multi", "start");
        let _ = engine.decide(&feat1);
        let feat2 = features_with_messages(
            "gpt-4o-multi",
            vec![
                ("user", "start"),
                ("assistant", "Turn 1: Hello!"),
                ("user", "continue"),
            ],
        );
        let _ = engine.decide(&feat2);

        // Turn 3
        let feat = features_with_messages(
            "gpt-4o-multi",
            vec![
                ("user", "start"),
                ("assistant", "Turn 1: Hello!"),
                ("user", "continue"),
                ("assistant", "Turn 2: How are you?"),
                ("user", "bye"),
            ],
        );
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(
                    d.response_blocks[0].content.as_deref(),
                    Some("Turn 3: Goodbye!")
                );
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    fn decide_fixture_usage_response() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // usage-response.json: usage-report with model "gpt-4o-usage"
        let feat = features_with_model("gpt-4o-usage", "hi");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(d.scenario, "usage-report");
                let u = d.usage.unwrap();
                assert_eq!(u.prompt_tokens, Some(15));
                assert_eq!(u.completion_tokens, Some(30));
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    fn decide_unknown_model_returns_default() {
        let dir = fixture_scenarios_dir();
        let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

        // No fixture matches model "unknown-model" -> default placeholder
        let feat = features_with_model("unknown-model", "hi");
        let outcome = engine.decide(&feat);
        match outcome {
            DecisionOutcome::Decision(d) => {
                assert_eq!(d.scenario, "default");
                assert_eq!(d.response_blocks[0].content.as_deref(), Some("placeholder"));
            }
            DecisionOutcome::Error(_) => panic!("expected decision"),
        }
    }

    #[test]
    #[should_panic(expected = "exceeded declared turns")]
    fn decide_panics_on_turn_overflow() {
        let scenario = ScenarioDeclaration {
            name: "single-turn".to_string(),
            match_: None,
            turns: vec![text_turn("only one")],
            models: None,
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);

        // First request → turn 0, succeeds.
        let feat1 = features("gpt-4", "hi");
        let _ = engine.decide(&feat1);

        // Second request with extended history → turn 1, exceeds max 1.
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
                    content: "only one".to_string(),
                },
                MessageEntry {
                    role: "user".to_string(),
                    content: "next".to_string(),
                },
            ],
            tools: vec![],
        };
        let _ = engine.decide(&feat2);
    }

    // ------------------------------------------------------------------
    // decide_for_models tests
    // ------------------------------------------------------------------

    #[test]
    fn decide_for_models_returns_scenario_declared_models() {
        let scenario = ScenarioDeclaration {
            name: "models-scene".to_string(),
            match_: None,
            turns: vec![text_turn("ok")],
            models: Some(vec![
                ModelEntry {
                    id: "gpt-4".to_string(),
                    owned_by: "openai".to_string(),
                },
                ModelEntry {
                    id: "claude-3".to_string(),
                    owned_by: "anthropic".to_string(),
                },
            ]),
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let decision = engine.decide_for_models();
        match decision {
            ModelsDecision::Models(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].id, "gpt-4");
                assert_eq!(entries[1].id, "claude-3");
            }
            _ => panic!("expected Models variant"),
        }
    }

    #[test]
    fn decide_for_models_placeholder_when_no_models_declared() {
        let scenario = ScenarioDeclaration {
            name: "no-models".to_string(),
            match_: None,
            turns: vec![text_turn("ok")],
            models: None,
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let decision = engine.decide_for_models();
        assert!(matches!(decision, ModelsDecision::Placeholder));
    }

    #[test]
    fn decide_for_models_placeholder_when_no_scenarios() {
        let mut engine = ScenarioEngine::new(vec![]);
        let decision = engine.decide_for_models();
        assert!(matches!(decision, ModelsDecision::Placeholder));
    }

    #[test]
    fn decide_for_models_error_injection() {
        let scenario = ScenarioDeclaration {
            name: "models-error".to_string(),
            match_: None,
            turns: vec![TurnResponse {
                response: ResponseShape::Text(TextResponse {
                    content: String::new(),
                }),
                delay: None,
                error: Some(HttpError {
                    status: 429,
                    message: "rate limited".to_string(),
                }),
            }],
            models: Some(vec![ModelEntry {
                id: "gpt-4".to_string(),
                owned_by: "openai".to_string(),
            }]),
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let decision = engine.decide_for_models();
        match decision {
            ModelsDecision::Error(e) => {
                assert_eq!(e.status, 429);
                assert_eq!(e.message, "rate limited");
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn decide_for_models_returns_models_when_no_error() {
        let scenario = ScenarioDeclaration {
            name: "models-ok".to_string(),
            match_: None,
            turns: vec![TurnResponse {
                response: ResponseShape::Text(TextResponse {
                    content: String::new(),
                }),
                delay: None,
                error: None,
            }],
            models: Some(vec![ModelEntry {
                id: "test-model".to_string(),
                owned_by: "test-org".to_string(),
            }]),
        };
        let mut engine = ScenarioEngine::new(vec![scenario]);
        let decision = engine.decide_for_models();
        match decision {
            ModelsDecision::Models(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].id, "test-model");
            }
            _ => panic!("expected Models variant"),
        }
    }
}
