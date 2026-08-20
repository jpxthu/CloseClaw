//! Request feature matcher.
//!
//! Matches incoming [`RequestFeatures`] against loaded scenario declarations
//! to select the appropriate scenario for a request. Uses a pre-built
//! model_id index for O(1) lookup per request, matching the design doc's
//! performance requirement.

use std::collections::HashMap;

use super::types::{MatchCondition, ScenarioDeclaration};
use crate::types::RequestFeatures;

/// Pre-built index for efficient scenario matching.
///
/// Created once from the loaded scenario list, then reused for every request.
/// Scenarios are grouped by `model_id` for O(1) lookup, with ungrouped
/// scenarios (no model constraint) checked against every request.
pub struct MatcherIndex {
    /// Indices of scenarios with no model_id constraint — checked against
    /// every request.
    any_model: Vec<usize>,
    /// Indices of scenarios grouped by model_id.
    by_model: HashMap<String, Vec<usize>>,
    /// Reference to the original scenario list.
    scenarios: Vec<ScenarioDeclaration>,
}

impl MatcherIndex {
    /// Build an index from a list of scenario declarations.
    pub fn build(scenarios: Vec<ScenarioDeclaration>) -> Self {
        let mut any_model = Vec::new();
        let mut by_model: HashMap<String, Vec<usize>> = HashMap::new();

        for (i, scenario) in scenarios.iter().enumerate() {
            match &scenario.match_ {
                None => {
                    any_model.push(i);
                }
                Some(cond) => match &cond.model_id {
                    Some(model_id) => {
                        by_model.entry(model_id.clone()).or_default().push(i);
                    }
                    None => {
                        any_model.push(i);
                    }
                },
            }
        }

        Self {
            any_model,
            by_model,
            scenarios,
        }
    }

    /// Match a request against the indexed scenarios.
    ///
    /// Returns the index of the matched scenario within the internal list,
    /// or `None` if nothing matches. Panics if multiple scenarios match —
    /// indicates a scenario file error.
    pub fn match_request(&self, features: &RequestFeatures) -> Option<usize> {
        let mut matched: Vec<usize> = Vec::new();

        // Check any-model bucket
        for &idx in &self.any_model {
            if scenario_matches(features, &self.scenarios[idx]) {
                matched.push(idx);
            }
        }

        // Check model-specific bucket
        if let Some(model_indices) = self.by_model.get(&features.model) {
            for &idx in model_indices {
                if scenario_matches(features, &self.scenarios[idx]) {
                    matched.push(idx);
                }
            }
        }

        matched.dedup();

        match matched.len() {
            0 => None,
            1 => Some(matched[0]),
            _ => {
                let names: Vec<&str> = matched
                    .iter()
                    .map(|&i| self.scenarios[i].name.as_str())
                    .collect();
                panic!(
                    "scenario file error: multiple scenarios matched request (model={}): {:?}",
                    features.model, names
                );
            }
        }
    }

    /// Get the scenario at the given index.
    pub fn get(&self, index: usize) -> &ScenarioDeclaration {
        &self.scenarios[index]
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
    if let Some(ref required_model) = condition.model_id {
        if features.model != *required_model {
            return false;
        }
    }

    if let Some(ref substr) = condition.message_contains {
        let found = features.messages.iter().any(|m| m.content.contains(substr));
        if !found {
            return false;
        }
    }

    if let Some(ref required_tool) = condition.tool_name {
        if !features.tools.iter().any(|t| t == required_tool) {
            return false;
        }
    }

    true
}

/// Convenience function: match a request against a list of scenario declarations.
///
/// Returns a reference to the matched scenario from the input slice, or `None`.
/// Prefer using [`MatcherIndex`] directly when matching multiple requests
/// against the same scenario list for better performance.
pub fn match_scenario<'a>(
    features: &RequestFeatures,
    scenarios: &'a [ScenarioDeclaration],
) -> Option<&'a ScenarioDeclaration> {
    let index = MatcherIndex::build(scenarios.to_vec());
    index.match_request(features).map(|i| &scenarios[i])
}

#[cfg(test)]
mod tests {
    use super::super::types::MessageEntry;
    use super::*;
    use crate::scenario::types::{ResponseShape, TextResponse, TurnResponse};

    fn fallback(name: &str) -> ScenarioDeclaration {
        ScenarioDeclaration {
            name: name.to_string(),
            match_: None,
            turns: vec![turn()],
        }
    }

    fn specific(name: &str, condition: MatchCondition) -> ScenarioDeclaration {
        ScenarioDeclaration {
            name: name.to_string(),
            match_: Some(condition),
            turns: vec![turn()],
        }
    }

    fn turn() -> TurnResponse {
        TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: "ok".to_string(),
            }),
            delay: None,
            error: None,
        }
    }

    fn feat(model: &str, messages: Vec<&str>, tools: Vec<&str>) -> RequestFeatures {
        RequestFeatures {
            model: model.to_string(),
            stream: false,
            max_tokens: None,
            temperature: None,
            messages: messages
                .into_iter()
                .map(|c| MessageEntry {
                    role: "user".to_string(),
                    content: c.to_string(),
                })
                .collect(),
            tools: tools.into_iter().map(String::from).collect(),
        }
    }

    // ------------------------------------------------------------------
    // MatcherIndex tests
    // ------------------------------------------------------------------

    #[test]
    fn index_exact_model_match() {
        let scenarios = vec![specific(
            "gpt4",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat("gpt-4o", vec!["hi"], vec![]));
        assert_eq!(index.get(result.unwrap()).name, "gpt4");
    }

    #[test]
    fn index_model_mismatch_returns_none() {
        let scenarios = vec![specific(
            "gpt4",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat("claude-3", vec!["hi"], vec![]));
        assert!(result.is_none());
    }

    #[test]
    fn index_message_contains_match() {
        let scenarios = vec![specific(
            "math",
            MatchCondition {
                message_contains: Some("calculate".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat("gpt-4o", vec!["please calculate 2+2"], vec![]));
        assert_eq!(index.get(result.unwrap()).name, "math");
    }

    #[test]
    fn index_message_contains_no_match() {
        let scenarios = vec![specific(
            "math",
            MatchCondition {
                message_contains: Some("calculate".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat("gpt-4o", vec!["hello world"], vec![]));
        assert!(result.is_none());
    }

    #[test]
    fn index_tool_name_match() {
        let scenarios = vec![specific(
            "web_search",
            MatchCondition {
                tool_name: Some("search".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat("gpt-4o", vec!["hi"], vec!["search", "math"]));
        assert_eq!(index.get(result.unwrap()).name, "web_search");
    }

    #[test]
    fn index_tool_name_no_match() {
        let scenarios = vec![specific(
            "web_search",
            MatchCondition {
                tool_name: Some("search".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat("gpt-4o", vec!["hi"], vec!["math"]));
        assert!(result.is_none());
    }

    #[test]
    fn index_combined_conditions_all_must_match() {
        let scenarios = vec![specific(
            "specific",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                message_contains: Some("hello".to_string()),
                tool_name: Some("search".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);

        let r = index.match_request(&feat("gpt-4o", vec!["hello world"], vec!["search"]));
        assert_eq!(index.get(r.unwrap()).name, "specific");

        assert!(index
            .match_request(&feat("claude-3", vec!["hello world"], vec!["search"]))
            .is_none());
        assert!(index
            .match_request(&feat("gpt-4o", vec!["goodbye"], vec!["search"]))
            .is_none());
        assert!(index
            .match_request(&feat("gpt-4o", vec!["hello world"], vec!["math"]))
            .is_none());
    }

    #[test]
    fn index_fallback_matches_any() {
        let scenarios = vec![fallback("fallback")];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat("any-model", vec!["anything"], vec![]));
        assert_eq!(index.get(result.unwrap()).name, "fallback");
    }

    #[test]
    #[should_panic(expected = "multiple scenarios matched")]
    fn index_fallback_and_specific_both_match_panics() {
        let scenarios = vec![
            specific(
                "specific",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
            fallback("fallback"),
        ];
        let index = MatcherIndex::build(scenarios);
        index.match_request(&feat("gpt-4o", vec!["hi"], vec![]));
    }

    #[test]
    #[should_panic(expected = "multiple scenarios matched")]
    fn index_multiple_specific_matches_panics() {
        let scenarios = vec![
            specific(
                "a",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
            specific(
                "b",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let index = MatcherIndex::build(scenarios);
        index.match_request(&feat("gpt-4o", vec!["hi"], vec![]));
    }

    #[test]
    fn index_no_match_returns_none() {
        let scenarios = vec![specific(
            "gpt4",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                message_contains: Some("special".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat("claude-3", vec!["hi"], vec![]));
        assert!(result.is_none());
    }

    #[test]
    fn index_empty_returns_none() {
        let index = MatcherIndex::build(vec![]);
        let result = index.match_request(&feat("gpt-4o", vec!["hi"], vec![]));
        assert!(result.is_none());
    }

    #[test]
    fn index_message_contains_checks_all_messages() {
        let scenarios = vec![specific(
            "math",
            MatchCondition {
                message_contains: Some("calculate".to_string()),
                ..Default::default()
            },
        )];
        let index = MatcherIndex::build(scenarios);
        let result = index.match_request(&feat(
            "gpt-4o",
            vec!["first message", "please calculate this"],
            vec![],
        ));
        assert_eq!(index.get(result.unwrap()).name, "math");
    }

    #[test]
    fn index_multiple_models_indexed_separately() {
        let scenarios = vec![
            specific(
                "gpt4-scene",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
            specific(
                "claude-scene",
                MatchCondition {
                    model_id: Some("claude-3-opus".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let index = MatcherIndex::build(scenarios);

        let r1 = index.match_request(&feat("gpt-4o", vec!["hi"], vec![]));
        assert_eq!(index.get(r1.unwrap()).name, "gpt4-scene");

        let r2 = index.match_request(&feat("claude-3-opus", vec!["hi"], vec![]));
        assert_eq!(index.get(r2.unwrap()).name, "claude-scene");

        let r3 = index.match_request(&feat("unknown-model", vec!["hi"], vec![]));
        assert!(r3.is_none());
    }

    // ------------------------------------------------------------------
    // match_scenario convenience function tests
    // ------------------------------------------------------------------

    #[test]
    fn convenience_match_scenario_works() {
        let scenarios = vec![specific(
            "gpt4",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        )];
        let result = match_scenario(&feat("gpt-4o", vec!["hi"], vec![]), &scenarios);
        assert_eq!(result.unwrap().name, "gpt4");
    }

    #[test]
    fn convenience_match_scenario_no_match() {
        let scenarios = vec![specific(
            "gpt4",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        )];
        let result = match_scenario(&feat("claude-3", vec!["hi"], vec![]), &scenarios);
        assert!(result.is_none());
    }

    #[test]
    fn convenience_match_scenario_fallback() {
        let scenarios = vec![fallback("fallback")];
        let result = match_scenario(&feat("any", vec!["hi"], vec![]), &scenarios);
        assert_eq!(result.unwrap().name, "fallback");
    }
}
