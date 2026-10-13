//! Request feature matcher.
//!
//! Matches incoming [`RequestFeatures`] against loaded scenario declarations
//! to select the appropriate scenario for a request. Uses a pre-built
//! model_id index for O(1) lookup per request, matching the design doc's
//! performance requirement.

use std::collections::HashMap;

use super::types::{MatchCondition, ScenarioDeclaration};
use crate::types::{ProtocolKind, RequestFeatures};

pub(crate) mod conflict;
pub use conflict::{detect_conflicts, ConflictReport, ScenarioConflictError};

/// Pre-built index for efficient scenario matching.
///
/// Created once from the loaded scenario list, then reused for every request.
/// Scenarios are grouped by `(model_id, protocol)` for O(1) lookup.
/// Scenarios without a model constraint are grouped by protocol only,
/// so that OpenAI requests never match Anthropic-only fallback buckets
/// and vice versa.
#[derive(Debug)]
pub struct MatcherIndex {
    /// Indices of scenarios with no model_id constraint, grouped by protocol.
    any_by_protocol: HashMap<ProtocolKind, Vec<usize>>,
    /// Indices of scenarios grouped by `(model_id, protocol)` composite key.
    by_model: HashMap<(String, ProtocolKind), Vec<usize>>,
    /// Index of the fallback scenario per protocol (match_ = None).
    /// At most one fallback per protocol (enforced by conflict detection).
    fallback_by_protocol: HashMap<ProtocolKind, usize>,
    /// Reference to the original scenario list.
    scenarios: Vec<ScenarioDeclaration>,
}

impl MatcherIndex {
    /// Build an index from a list of scenario declarations.
    /// Returns an error if conflict detection finds scenarios that could
    /// both match the same request (multi-hit at startup).
    pub fn build(scenarios: Vec<ScenarioDeclaration>) -> Result<Self, ScenarioConflictError> {
        let mut any_by_protocol: HashMap<ProtocolKind, Vec<usize>> = HashMap::new();
        let mut by_model: HashMap<(String, ProtocolKind), Vec<usize>> = HashMap::new();
        for (i, scenario) in scenarios.iter().enumerate() {
            match &scenario.match_ {
                // Fallback scenarios are NOT placed in any_by_protocol —
                // they are only checked as the last-resort兜底 via
                // fallback_by_protocol.
                None => {}
                Some(cond) => match &cond.model_id {
                    Some(model_id) => {
                        for proto in [ProtocolKind::OpenAi, ProtocolKind::Anthropic] {
                            by_model
                                .entry((model_id.clone(), proto))
                                .or_default()
                                .push(i);
                        }
                    }
                    None => {
                        for proto in [ProtocolKind::OpenAi, ProtocolKind::Anthropic] {
                            any_by_protocol.entry(proto).or_default().push(i);
                        }
                    }
                },
            }
        }
        let conflicts = conflict::detect_conflicts(&scenarios);
        if !conflicts.is_empty() {
            return Err(ScenarioConflictError { conflicts });
        }
        Ok(Self {
            any_by_protocol,
            by_model,
            fallback_by_protocol: Self::build_fallback_index(&scenarios),
            scenarios,
        })
    }

    /// Build fallback index: one fallback scenario per protocol (match_ = None).
    fn build_fallback_index(scenarios: &[ScenarioDeclaration]) -> HashMap<ProtocolKind, usize> {
        let mut fallback: HashMap<ProtocolKind, usize> = HashMap::new();
        for (i, scenario) in scenarios.iter().enumerate() {
            if scenario.match_.is_none() {
                for proto in [ProtocolKind::OpenAi, ProtocolKind::Anthropic] {
                    fallback.entry(proto).or_insert(i);
                }
            }
        }
        fallback
    }

    /// Match a request against the indexed scenarios.
    /// Returns the index of the matched scenario, or `None` if nothing matches.
    /// Conflict detection at build time guarantees at most one match.
    pub fn match_request(&self, features: &RequestFeatures) -> Option<usize> {
        let key = (features.model.clone(), features.protocol);
        // Check any-model bucket for this protocol only
        if let Some(indices) = self.any_by_protocol.get(&features.protocol) {
            if let Some(&idx) = indices
                .iter()
                .find(|&&i| scenario_matches(features, &self.scenarios[i]))
            {
                return Some(idx);
            }
        }
        // Check model-specific bucket for (model, protocol)
        if let Some(indices) = self.by_model.get(&key) {
            if let Some(&idx) = indices
                .iter()
                .find(|&&i| scenario_matches(features, &self.scenarios[i]))
            {
                return Some(idx);
            }
        }
        self.fallback_by_protocol.get(&features.protocol).copied()
    }

    /// Get the scenario at the given index.
    pub fn get(&self, index: usize) -> &ScenarioDeclaration {
        &self.scenarios[index]
    }

    /// Get the number of loaded scenarios.
    pub fn len(&self) -> usize {
        self.scenarios.len()
    }

    /// Check if there are no loaded scenarios.
    pub fn is_empty(&self) -> bool {
        self.scenarios.is_empty()
    }
}

/// Check whether a scenario's conditions are satisfied by the request.
fn scenario_matches(features: &RequestFeatures, scenario: &ScenarioDeclaration) -> bool {
    match &scenario.match_ {
        None => true,
        Some(condition) => match_condition_satisfied(features, condition),
    }
}

/// Check whether all specified fields in a match condition are satisfied.
fn match_condition_satisfied(features: &RequestFeatures, condition: &MatchCondition) -> bool {
    if !matches_model_id(features, condition) {
        return false;
    }
    if !matches_message_contains(features, condition) {
        return false;
    }
    if !matches_tool_name(features, condition) {
        return false;
    }
    if !matches_request_params(features, condition) {
        return false;
    }
    true
}

/// Check if the request model matches the condition's `model_id`.
fn matches_model_id(features: &RequestFeatures, condition: &MatchCondition) -> bool {
    match &condition.model_id {
        Some(required) => features.model == *required,
        None => true,
    }
}

/// Check if at least one message contains the condition's
/// `message_contains` substring.
fn matches_message_contains(features: &RequestFeatures, condition: &MatchCondition) -> bool {
    match &condition.message_contains {
        Some(substr) => features.messages.iter().any(|m| m.content.contains(substr)),
        None => true,
    }
}

/// Check if the request tools contain the condition's `tool_name`.
fn matches_tool_name(features: &RequestFeatures, condition: &MatchCondition) -> bool {
    match &condition.tool_name {
        Some(required) => features.tools.iter().any(|t| t == required),
        None => true,
    }
}

/// Check if the request parameters match the condition's `request_params`.
///
/// Each key-value pair in `request_params` is compared against the
/// corresponding field in [`RequestFeatures`]. Unknown keys are silently
/// ignored. When `request_params` is `None`, the check always passes.
fn matches_request_params(features: &RequestFeatures, condition: &MatchCondition) -> bool {
    let params = match &condition.request_params {
        Some(p) => p,
        None => return true,
    };
    params.iter().all(|(key, expected)| {
        let actual = features_to_json_value(features, key.as_str());
        match actual {
            Some(v) => {
                if key == "temperature" {
                    json_value_matches_temperature(&v, expected)
                } else {
                    v == *expected
                }
            }
            None => true, // unknown key => ignore
        }
    })
}

/// Map a [`RequestFeatures`] field to a JSON value by key name.
///
/// Returns `None` for unknown keys (treated as non-constraining).
///
/// For `temperature` (f32), the value is stored as `serde_json::Number`
/// and compared separately in [`matches_request_params`] using `f32`
/// precision to avoid mismatches between `f32` serialization and `f64`
/// JSON parsing.
fn features_to_json_value(features: &RequestFeatures, key: &str) -> Option<serde_json::Value> {
    match key {
        "stream" => Some(serde_json::Value::Bool(features.stream)),
        "max_tokens" => features.max_tokens.map(|v| serde_json::json!(v)),
        "temperature" => features.temperature.map(|v| serde_json::json!(v)),
        _ => None,
    }
}

/// Compare two JSON values with f32-appropriate precision for temperature.
///
/// When comparing `temperature`, both values are first compared as `f32`
/// to avoid precision mismatches between the f32 feature value and the
/// f64 value parsed from the scenario file.
fn json_value_matches_temperature(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
) -> bool {
    if let (Some(a), Some(b)) = (actual.as_f64(), expected.as_f64()) {
        // Compare as f32 to match the feature's native precision
        (a as f32) == (b as f32)
    } else {
        actual == expected
    }
}

/// Convenience function: match a request against a list of scenario
/// declarations.
///
/// Returns a reference to the matched scenario from the input slice, or
/// `None`. Prefer using [`MatcherIndex`] directly when matching multiple
/// requests against the same scenario list for better performance.
pub fn match_scenario<'a>(
    features: &RequestFeatures,
    scenarios: &'a [ScenarioDeclaration],
) -> Result<Option<&'a ScenarioDeclaration>, ScenarioConflictError> {
    let index = MatcherIndex::build(scenarios.to_vec())?;
    Ok(index.match_request(features).map(|i| &scenarios[i]))
}

#[cfg(test)]
mod mod_tests;

#[cfg(test)]
mod conflict_tests;

#[cfg(test)]
mod matcher_integration_tests;

#[cfg(test)]
mod matcher_request_params_tests;
