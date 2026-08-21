//! Scenario engine — core decision-making for Fake LLM Server.
//!
//! This module implements the scenario matching, session tracking, and
//! response generation pipeline. See `docs/design/fake_llm/scenario-engine.md`.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use std::collections::HashMap;

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
#[derive(Debug)]
pub enum ModelsDecision {
    /// Scenario-declared model list with optional delay.
    Models(Vec<ModelEntry>, Option<u64>),
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
    /// Per-scenario KV cache simulators, keyed by scenario name.
    kv_caches: HashMap<String, KvCacheSimulator>,
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
            kv_caches: HashMap::new(),
        })
    }

    /// Create an engine with an explicit scenario list (for testing).
    pub fn new(scenarios: Vec<ScenarioDeclaration>) -> Self {
        let matcher = MatcherIndex::build(scenarios);
        Self {
            matcher,
            sessions: SessionTracker::new(),
            kv_caches: HashMap::new(),
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
        // Each scenario has its own KvCacheSimulator for state isolation.
        let explicit_hit = usage.as_ref().and_then(|u| u.cache_hit_tokens);
        let explicit_write = usage.as_ref().and_then(|u| u.cache_write_tokens);
        let kv_cache = self.kv_caches.entry(matched.name.clone()).or_default();
        let cache_result = kv_cache.process(
            &matched.name,
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
            first_token_delay: turn_resp.first_token_delay,
            segment_delay: turn_resp.segment_delay,
            stream_interrupt_after: turn_resp.stream_interrupt_after,
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
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        })
    }

    /// Decide how to respond to a `/v1/models` request.
    ///
    /// Iterates through all loaded scenarios to find the first one that
    /// declares a `models` list. If the current turn for that scenario has
    /// an error injection, that takes priority. If no scenario declares
    /// models, returns `Placeholder` (caller uses the default model list).
    pub fn decide_for_models(&mut self) -> ModelsDecision {
        // Iterate all scenarios to find the first one with a `models` field.
        // The matcher may fail for models requests because they have no
        // request body with a model ID, so we scan directly.
        for i in 0..self.matcher.len() {
            let matched = self.matcher.get(i);
            if matched.models.is_none() {
                continue;
            }

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

            // Advance KV cache state for this scenario (models endpoint
            // still needs to track prefix state for consistency).
            let kv_cache = self.kv_caches.entry(matched.name.clone()).or_default();
            let empty_msgs = vec![];
            let _ = kv_cache.process(&matched.name, &empty_msgs, &[], None, None);

            if let Some(ref models) = matched.models {
                return ModelsDecision::Models(models.clone(), turn_resp.delay);
            }
        }

        ModelsDecision::Placeholder
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
            ResponseShape::Reasoning(r) => {
                let reasoning_text =
                    Self::generate_reasoning_by_intensity(&r.reasoning, &r.intensity);
                vec![
                    ResponseBlock {
                        block_type: "reasoning".to_string(),
                        content: None,
                        tool_name: None,
                        tool_arguments: None,
                        reasoning: Some(reasoning_text),
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
                ]
            }
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

    /// Generate reasoning text by intensity level.
    ///
    /// - Low: short reasoning (~50 chars)
    /// - Medium: moderate reasoning (~150 chars, the default)
    /// - High: lengthy reasoning (~300 chars)
    ///
    /// If the input reasoning is non-empty, it is used as the base
    /// and extended according to the intensity level.
    fn generate_reasoning_by_intensity(reasoning: &str, intensity: &ReasoningIntensity) -> String {
        if reasoning.is_empty() {
            return String::new();
        }
        match intensity {
            ReasoningIntensity::Low => format!("Let me think briefly. {}", reasoning,),
            ReasoningIntensity::Medium => format!(
                "Let me consider this carefully. {} I need to verify each step.",
                reasoning,
            ),
            ReasoningIntensity::High => format!(
                "Let me think through this problem in detail. {} \
First, I should analyze the input. Then I'll evaluate the options. \
Next, I'll consider edge cases and potential pitfalls. \
Finally, I'll synthesize a comprehensive answer.",
                reasoning,
            ),
        }
    }

    /// Extract usage from a response shape, if present.
    fn extract_usage(shape: &ResponseShape) -> Option<UsageResponse> {
        match shape {
            ResponseShape::Usage(u) => Some(u.clone()),
            ResponseShape::Text(t) => t.usage.clone(),
            ResponseShape::Reasoning(r) => r.usage.clone(),
            ResponseShape::ToolCall(tc) => tc.usage.clone(),
            _ => None,
        }
    }

    /// Merge KV cache simulation results into usage fields.
    ///
    /// When `cache_fields_missing` is true on the usage, auto-simulated
    /// cache fields are not filled (the provider is declared to not return
    /// cache fields). Explicit injection values are always preserved.
    ///
    /// The state machine (`kv_cache.process`) is always called by the
    /// caller to maintain internal state regardless of this flag.
    fn merge_cache_into_usage(
        usage: &mut Option<UsageResponse>,
        cache: &crate::kv_cache::CacheResult,
    ) {
        let u = usage.get_or_insert_with(UsageResponse::default);
        if u.cache_fields_missing {
            return;
        }
        if cache.cache_hit_tokens.is_none() && cache.cache_write_tokens.is_none() {
            return;
        }
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
mod tests;
