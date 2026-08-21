//! Scenario engine — core decision-making for Fake LLM Server.
//!
//! This module implements the scenario matching, session tracking, and
//! response generation pipeline. See `docs/design/fake_llm/scenario-engine.md`.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use std::collections::HashMap;

use std::time::Duration;

use crate::kv_cache::KvCacheSimulator;
use crate::types::{RequestFeatures, ScenarioDecision};

pub mod loader;
pub mod matcher;
pub mod session;
pub mod types;

pub use matcher::MatcherIndex;
pub use matcher::{detect_conflicts, ScenarioConflictError};
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
    /// Total number of `decide()` calls since engine creation.
    request_count: usize,
}

/// How often (in `decide()` calls) to trigger session cleanup.
const CLEANUP_INTERVAL: usize = 100;

/// Default session TTL for automatic cleanup (30 minutes).
const SESSION_TTL: Duration = Duration::from_secs(1800);

impl ScenarioEngine {
    /// Create a new engine by loading scenario files from the given directory.
    pub fn from_dir(dir: &std::path::Path) -> Result<Self> {
        let mut all_scenarios = Vec::new();
        let files = load_scenario_dir(dir)
            .with_context(|| format!("loading scenario dir: {}", dir.display()))?;
        for file in files {
            all_scenarios.extend(file.scenarios);
        }
        let matcher = MatcherIndex::build(all_scenarios).context("conflict detection failed")?;
        Ok(Self {
            matcher,
            sessions: SessionTracker::new(),
            kv_caches: HashMap::new(),
            request_count: 0,
        })
    }

    /// Create an engine with an explicit scenario list (for testing).
    ///
    /// Returns an error if the scenarios contain conflicts.
    pub fn new(scenarios: Vec<ScenarioDeclaration>) -> Result<Self, ScenarioConflictError> {
        let matcher = MatcherIndex::build(scenarios)?;
        Ok(Self {
            matcher,
            sessions: SessionTracker::new(),
            kv_caches: HashMap::new(),
            request_count: 0,
        })
    }

    /// Decide how to respond to the given request features.
    ///
    /// Flow: match scenario → advance session turn → build decision.
    ///
    /// When no scenario matches and no fallback is declared, returns
    /// an HTTP 500 error (all behavior comes from scenarios — no implicit
    /// placeholder responses).
    pub fn decide(&mut self, features: &RequestFeatures) -> DecisionOutcome {
        self.maybe_cleanup();

        let matched_idx = match self.matcher.match_request(features) {
            Some(idx) => idx,
            None => {
                return DecisionOutcome::Error(HttpError {
                    status: 500,
                    message: "no scenario matched and no fallback declared".to_string(),
                    retry_after: None,
                });
            }
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
            return DecisionOutcome::Error(HttpError {
                status: 500,
                message: format!(
                    "scenario '{}' exceeded declared turns \
                     (turn {}, max {})",
                    matched.name, turn, max_turns
                ),
                retry_after: None,
            });
        }

        let turn_resp = matched.turns[turn].clone();
        if let Some(error) = &turn_resp.error {
            return DecisionOutcome::Error(error.clone());
        }

        let scenario_name = matched.name.clone();
        self.build_decision(features, &scenario_name, &turn_resp)
    }

    /// Trigger session cleanup if the request count is a multiple of
    /// the cleanup interval.
    fn maybe_cleanup(&mut self) {
        self.request_count += 1;
        if self.request_count.is_multiple_of(CLEANUP_INTERVAL) {
            self.sessions.cleanup_expired(SESSION_TTL);
        }
    }

    /// Build the final decision from matched scenario and turn response.
    ///
    /// Shape-level delivery control is extracted here:
    /// - `Error` shape → `DecisionOutcome::Error` (TurnResponse.error takes
    ///   priority when both are present; see `decide` above).
    /// - `Streaming` / `Delay` shapes → delay and granularity parameters are
    ///   merged into the decision only when the corresponding TurnResponse
    ///   field is `None`.
    fn build_decision(
        &mut self,
        features: &RequestFeatures,
        scenario_name: &str,
        turn_resp: &TurnResponse,
    ) -> DecisionOutcome {
        let shapes = turn_resp.response.to_shapes();

        // --- Error shape: early return ---
        // TurnResponse.error is already handled in `decide()` with higher
        // priority. If we are here, TurnResponse.error was None.
        if let Some(http_err) = Self::extract_error_shape(&shapes) {
            return DecisionOutcome::Error(http_err);
        }

        // --- Extract shape-level delivery parameters ---
        // Only fill when TurnResponse did not explicitly set the field.
        // NOTE: segment_granularity is only declared by shapes; there is no
        // corresponding TurnResponse field, so it has no override source.
        let (delay, first_token_delay, segment_delay, segment_granularity) =
            Self::extract_delivery_params(&shapes, turn_resp);

        let blocks = Self::build_response_blocks(&shapes);
        let mut usage = Self::extract_usage(&shapes);
        // KV cache simulation: compute cache fields and merge.
        // Explicit injection takes priority over auto.
        let explicit_hit = usage.as_ref().and_then(|u| u.cache_hit_tokens);
        let explicit_write = usage.as_ref().and_then(|u| u.cache_write_tokens);
        let kv_cache = self.kv_caches.entry(scenario_name.to_string()).or_default();
        let cache_result = kv_cache.process(
            scenario_name,
            &features.messages,
            &features.tools,
            explicit_hit,
            explicit_write,
        );
        Self::merge_cache_into_usage(&mut usage, &cache_result);
        DecisionOutcome::Decision(ScenarioDecision {
            model: features.model.clone(),
            scenario: scenario_name.to_string(),
            stream: features.stream,
            response_blocks: blocks,
            http_error: None,
            delay,
            first_token_delay,
            segment_delay,
            stream_interrupt_after: turn_resp.stream_interrupt_after,
            segment_granularity,
            usage,
        })
    }

    /// Extract an error shape from the response shapes, if present.
    ///
    /// Returns the first `ResponseShape::Error` encountered, converted
    /// into an `HttpError` for the decision outcome.
    fn extract_error_shape(shapes: &[ResponseShape]) -> Option<HttpError> {
        for shape in shapes {
            if let ResponseShape::Error(e) = shape {
                return Some(HttpError {
                    status: e.status,
                    message: e.message.clone(),
                    retry_after: e.retry_after,
                });
            }
        }
        None
    }

    /// Extract shape-level delivery parameters (delays + granularity).
    ///
    /// Iterates through shapes and fills in delivery parameters only
    /// when the corresponding `TurnResponse` field is `None`.
    /// `segment_granularity` is only declared by `Streaming` shapes;
    /// there is no corresponding `TurnResponse` field.
    fn extract_delivery_params(
        shapes: &[ResponseShape],
        turn_resp: &TurnResponse,
    ) -> (Option<u64>, Option<u64>, Option<u64>, Option<usize>) {
        let mut delay = turn_resp.delay;
        let mut first_token_delay = turn_resp.first_token_delay;
        let mut segment_delay = turn_resp.segment_delay;
        let mut segment_granularity: Option<usize> = None;

        for shape in shapes {
            match shape {
                ResponseShape::Streaming(s) => {
                    if segment_delay.is_none() {
                        segment_delay = s.segment_delay_ms;
                    }
                    if segment_granularity.is_none() {
                        segment_granularity = s.segment_granularity;
                    }
                }
                ResponseShape::Delay(d) => {
                    if delay.is_none() {
                        delay = d.delay_ms;
                    }
                    if first_token_delay.is_none() {
                        first_token_delay = d.first_token_delay_ms;
                    }
                    if segment_delay.is_none() {
                        segment_delay = d.segment_delay_ms;
                    }
                }
                _ => {}
            }
        }

        (delay, first_token_delay, segment_delay, segment_granularity)
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
                return ModelsDecision::Error(HttpError {
                    status: 500,
                    message: format!(
                        "scenario '{}' exceeded declared turns (turn {}, max {})",
                        matched.name, turn, max_turns
                    ),
                    retry_after: None,
                });
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

    /// Build response blocks from a slice of response shapes.
    ///
    /// Handles `Composite` variants by recursively flattening them.
    fn build_response_blocks(shapes: &[ResponseShape]) -> Vec<ResponseBlock> {
        let mut blocks = Vec::new();
        for shape in shapes {
            match shape {
                ResponseShape::Composite(inner) => {
                    blocks.extend(Self::build_response_blocks(inner));
                }
                ResponseShape::Text(t) => blocks.push(Self::build_text_block(&t.content)),
                ResponseShape::Reasoning(r) => {
                    blocks.extend(Self::build_reasoning_blocks(r));
                }
                ResponseShape::ToolCall(tc) => {
                    blocks.extend(Self::build_tool_call_blocks(tc));
                }
                // Control-only shapes produce no response blocks —
                // Streaming and Delay only affect delivery parameters.
                // Error is handled before block construction (early return).
                ResponseShape::Streaming(_) | ResponseShape::Delay(_) | ResponseShape::Error(_) => {
                }
                // Usage-only shapes produce no response blocks — only usage data.
                ResponseShape::Usage(_) => {}
                // Unknown shapes fall back to empty text block for backward compat.
                ResponseShape::Unknown => blocks.push(Self::build_text_block("")),
            }
        }
        blocks
    }

    /// Build a single text response block.
    fn build_text_block(content: &str) -> ResponseBlock {
        ResponseBlock {
            block_type: "text".to_string(),
            content: Some(content.to_string()),
            tool_name: None,
            tool_arguments: None,
            reasoning: None,
            signature: None,
        }
    }

    /// Build reasoning + text blocks from a reasoning shape.
    fn build_reasoning_blocks(r: &ReasoningResponse) -> Vec<ResponseBlock> {
        let reasoning_text = Self::generate_reasoning_by_intensity(&r.reasoning, &r.intensity);
        vec![
            ResponseBlock {
                block_type: "reasoning".to_string(),
                content: None,
                tool_name: None,
                tool_arguments: None,
                reasoning: Some(reasoning_text),
                signature: r.signature.clone(),
            },
            Self::build_text_block(&r.content),
        ]
    }

    /// Build tool call blocks from a tool call shape.
    fn build_tool_call_blocks(tc: &ToolCallResponse) -> Vec<ResponseBlock> {
        tc.calls
            .iter()
            .map(|call| ResponseBlock {
                block_type: "tool_call".to_string(),
                content: None,
                tool_name: Some(call.name.clone()),
                tool_arguments: Some(call.arguments.clone()),
                reasoning: None,
                signature: None,
            })
            .collect()
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

    /// Extract usage from a slice of response shapes.
    ///
    /// Returns the first `UsageResponse` found by iterating through shapes
    /// in order: top-level `Usage` variants, then embedded usage fields in
    /// `Text`, `Reasoning`, `ToolCall`, and `Streaming` variants.
    fn extract_usage(shapes: &[ResponseShape]) -> Option<UsageResponse> {
        for shape in shapes {
            let found = match shape {
                ResponseShape::Usage(u) => Some(u.clone()),
                ResponseShape::Text(t) => t.usage.clone(),
                ResponseShape::Reasoning(r) => r.usage.clone(),
                ResponseShape::ToolCall(tc) => tc.usage.clone(),
                ResponseShape::Streaming(s) => s.usage.clone(),
                ResponseShape::Composite(inner) => Self::extract_usage(inner),
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        None
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
