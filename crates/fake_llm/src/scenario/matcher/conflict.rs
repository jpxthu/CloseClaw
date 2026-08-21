//! Startup conflict detection for scenario declarations.
//!
//! When loading scenario files, we must detect pairs of scenarios that
//! could both match the same request (multi-hit). This is a scenario
//! file error and must be caught at build time, not at runtime.

use std::fmt;

use super::super::types::{MatchCondition, ScenarioDeclaration};

/// A report describing a conflict between two scenarios.
#[derive(Debug, Clone)]
pub struct ConflictReport {
    /// Name of the first conflicting scenario.
    pub scenario_a: String,
    /// Name of the second conflicting scenario.
    pub scenario_b: String,
    /// Human-readable reason for the conflict.
    pub reason: String,
}

impl fmt::Display for ConflictReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "conflict between '{}' and '{}': {}",
            self.scenario_a, self.scenario_b, self.reason
        )
    }
}

/// Error returned when startup conflict detection finds overlapping scenarios.
#[derive(Debug, thiserror::Error)]
#[error("scenario file error: {} conflicts detected", .conflicts.len())]
pub struct ScenarioConflictError {
    /// All detected conflicts.
    pub conflicts: Vec<ConflictReport>,
}

/// Detect conflicts among a list of scenario declarations.
///
/// Two scenarios conflict if there exists a request that could match both.
/// This is checked via exhaustive pairwise comparison of match conditions.
///
/// Fallback scenarios (match_ = None) match everything, so:
/// - Two fallbacks always conflict.
/// - A fallback + any conditional scenario always conflict.
pub fn detect_conflicts(scenarios: &[ScenarioDeclaration]) -> Vec<ConflictReport> {
    let mut conflicts = Vec::new();

    for i in 0..scenarios.len() {
        for j in (i + 1)..scenarios.len() {
            if let Some(report) = pair_conflicts(&scenarios[i], &scenarios[j]) {
                conflicts.push(report);
            }
        }
    }

    conflicts
}

/// Check if two scenarios conflict (could both match the same request).
fn pair_conflicts(a: &ScenarioDeclaration, b: &ScenarioDeclaration) -> Option<ConflictReport> {
    let conflict_reason = match (&a.match_, &b.match_) {
        // Both fallback: always conflict
        (None, None) => Some("both are fallback scenarios (no match conditions)".to_string()),
        // One fallback + one conditional: always conflict
        (None, Some(_)) | (Some(_), None) => {
            Some("fallback scenario conflicts with conditional scenario".to_string())
        }
        // Both conditional: check field-by-field compatibility
        (Some(cond_a), Some(cond_b)) => {
            if conditions_compatible(cond_a, cond_b) {
                Some("match conditions are simultaneously satisfiable".to_string())
            } else {
                None
            }
        }
    };

    conflict_reason.map(|reason| ConflictReport {
        scenario_a: a.name.clone(),
        scenario_b: b.name.clone(),
        reason,
    })
}

/// Check if two match conditions can both be satisfied by the same request.
///
/// Returns `true` if there exists a request that matches both conditions.
fn conditions_compatible(a: &MatchCondition, b: &MatchCondition) -> bool {
    compatible_model_id(a, b)
        && compatible_message_contains(a, b)
        && compatible_tool_name(a, b)
        && compatible_request_params(a, b)
}

/// Check if model_id constraints are compatible.
///
/// Both missing: compatible.
/// One missing: compatible (the other is still constrained).
/// Both present, equal: compatible.
/// Both present, unequal: incompatible.
fn compatible_model_id(a: &MatchCondition, b: &MatchCondition) -> bool {
    match (&a.model_id, &b.model_id) {
        (None, _) | (_, None) => true,
        (Some(id_a), Some(id_b)) => id_a == id_b,
    }
}

/// Check if message_contains constraints are compatible.
///
/// Both missing: compatible.
/// One missing: compatible.
/// Both present: compatible if one substring is contained in the other
/// (or equal). If neither is a substring of the other, no single message
/// can satisfy both simultaneously (conservative approximation).
fn compatible_message_contains(a: &MatchCondition, b: &MatchCondition) -> bool {
    match (&a.message_contains, &b.message_contains) {
        (None, _) | (_, None) => true,
        (Some(a_val), Some(b_val)) => {
            a_val.contains(b_val.as_str()) || b_val.contains(a_val.as_str())
        }
    }
}

/// Check if tool_name constraints are compatible.
///
/// Both missing: compatible.
/// One missing: compatible.
/// Both present, equal: compatible.
/// Both present, unequal: incompatible.
fn compatible_tool_name(a: &MatchCondition, b: &MatchCondition) -> bool {
    match (&a.tool_name, &b.tool_name) {
        (None, _) | (_, None) => true,
        (Some(name_a), Some(name_b)) => name_a == name_b,
    }
}

/// Check if request_params constraints are compatible.
///
/// Both missing/empty: compatible.
/// For each key present in either condition:
/// - If both set the same key with different values: incompatible.
/// - If only one sets a key: compatible (the other has no constraint on it).
/// - If both set the same key with equal values: compatible.
fn compatible_request_params(a: &MatchCondition, b: &MatchCondition) -> bool {
    let params_a = a.request_params.as_ref();
    let params_b = b.request_params.as_ref();

    match (params_a, params_b) {
        (None, _) | (_, None) => true,
        (Some(pa), Some(pb)) => {
            // Check all keys from both sides
            let all_keys: std::collections::HashSet<&String> = pa.keys().chain(pb.keys()).collect();
            all_keys.iter().all(|key| {
                match (pa.get(*key), pb.get(*key)) {
                    (Some(va), Some(vb)) => {
                        // Both set the same key: check value compatibility
                        json_values_compatible(va, vb)
                    }
                    // Only one sets it: compatible
                    _ => true,
                }
            })
        }
    }
}

/// Check if two JSON values are compatible (equal).
///
/// For temperature, comparison uses f32 precision to match the feature's
/// native type. For other values, exact JSON equality is used.
fn json_values_compatible(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    if let (Some(a_f64), Some(b_f64)) = (a.as_f64(), b.as_f64()) {
        // Compare as f32 to match feature precision
        (a_f64 as f32) == (b_f64 as f32)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallback(name: &str) -> ScenarioDeclaration {
        ScenarioDeclaration {
            name: name.to_string(),
            match_: None,
            turns: vec![],
            models: None,
        }
    }

    fn conditional(name: &str, condition: MatchCondition) -> ScenarioDeclaration {
        ScenarioDeclaration {
            name: name.to_string(),
            match_: Some(condition),
            turns: vec![],
            models: None,
        }
    }

    // ---------------------------------------------------------------
    // Fallback conflicts
    // ---------------------------------------------------------------

    #[test]
    fn two_fallbacks_conflict() {
        let scenarios = vec![fallback("a"), fallback("b")];
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].reason.contains("fallback"));
    }

    #[test]
    fn fallback_and_conditional_conflict() {
        let scenarios = vec![
            fallback("fallback"),
            conditional(
                "specific",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    // ---------------------------------------------------------------
    // model_id compatibility
    // ---------------------------------------------------------------

    #[test]
    fn same_model_id_conflicts() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn different_model_id_no_conflict() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    model_id: Some("claude-3".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn one_model_id_missing_conflict() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    model_id: None,
                    message_contains: Some("hello".to_string()),
                    ..Default::default()
                },
            ),
        ];
        // model_id: one missing => compatible
        // message_contains: one missing => compatible
        // Both can be satisfied by a request with model=gpt-4o and
        // message containing "hello" => conflict!
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    // ---------------------------------------------------------------
    // message_contains compatibility
    // ---------------------------------------------------------------

    #[test]
    fn same_message_contains_conflicts() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    message_contains: Some("hello".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    message_contains: Some("hello".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn substring_message_contains_conflicts() {
        // "calculate" is a substring of "calculate 2" — compatible
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    message_contains: Some("calculate".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    message_contains: Some("calculate 2".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn disjoint_message_contains_no_conflict() {
        // "foo" is not a substring of "bar" and vice versa
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    message_contains: Some("foo".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    message_contains: Some("bar".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert!(conflicts.is_empty());
    }

    // ---------------------------------------------------------------
    // tool_name compatibility
    // ---------------------------------------------------------------

    #[test]
    fn same_tool_name_conflicts() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    tool_name: Some("search".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    tool_name: Some("search".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn different_tool_name_no_conflict() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    tool_name: Some("search".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    tool_name: Some("code_exec".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert!(conflicts.is_empty());
    }

    // ---------------------------------------------------------------
    // request_params compatibility
    // ---------------------------------------------------------------

    #[test]
    fn same_request_params_conflicts() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    request_params: Some(
                        vec![("stream".to_string(), serde_json::json!(true))]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    request_params: Some(
                        vec![("stream".to_string(), serde_json::json!(true))]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn different_request_params_value_no_conflict() {
        // Different temperature values: mutually exclusive, no conflict.
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    request_params: Some(
                        vec![("temperature".to_string(), serde_json::json!(0.5))]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    request_params: Some(
                        vec![("temperature".to_string(), serde_json::json!(0.7))]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn one_key_missing_in_request_params_conflict() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    request_params: Some(
                        vec![("stream".to_string(), serde_json::json!(true))]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    request_params: Some(
                        vec![("max_tokens".to_string(), serde_json::json!(1024))]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            ),
        ];
        // A requires stream=true, B requires max_tokens=1024.
        // A request with stream=true AND max_tokens=1024 matches both => conflict.
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn mixed_request_params_one_key_same_one_different_no_conflict() {
        // stream matches in both, but temperature differs: mutually
        // exclusive due to temperature, no conflict.
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    request_params: Some(
                        vec![
                            ("stream".to_string(), serde_json::json!(true)),
                            ("temperature".to_string(), serde_json::json!(0.5)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    request_params: Some(
                        vec![
                            ("stream".to_string(), serde_json::json!(true)),
                            ("temperature".to_string(), serde_json::json!(0.9)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert!(conflicts.is_empty());
    }

    // ---------------------------------------------------------------
    // No conflict scenarios
    // ---------------------------------------------------------------

    #[test]
    fn completely_different_conditions_no_conflict() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    message_contains: Some("hello".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    model_id: Some("claude-3".to_string()),
                    message_contains: Some("world".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn no_conflicts_empty_list() {
        let conflicts = detect_conflicts(&[]);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn no_conflicts_single_scenario() {
        let scenarios = vec![conditional(
            "only",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        )];
        let conflicts = detect_conflicts(&scenarios);
        assert!(conflicts.is_empty());
    }

    // ---------------------------------------------------------------
    // Cross-protocol isolation (same model, different protocol)
    // ---------------------------------------------------------------

    #[test]
    fn cross_protocol_same_model_not_a_conflict_in_detection() {
        // Conflict detection operates on ScenarioDeclaration which has no
        // protocol field — protocol isolation is enforced by the index
        // bucketing in MatcherIndex. Two scenarios with the same model_id
        // but targeting different protocols would both appear in both
        // protocol buckets. The conflict detection doesn't know about
        // protocol, so same-model_id scenarios DO conflict at the
        // declaration level — the index prevents runtime overlap.
        //
        // However, if both scenarios have model_id="gpt-4o" with the same
        // message_contains, they ARE a conflict regardless of protocol
        // intent (both would match in both buckets).
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    message_contains: Some("hello".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    message_contains: Some("hello".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }

    // ---------------------------------------------------------------
    // Multiple conflicts
    // ---------------------------------------------------------------

    #[test]
    fn multiple_conflicts_detected() {
        let scenarios = vec![
            fallback("fb"),
            conditional(
                "a",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
            conditional(
                "b",
                MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    ..Default::default()
                },
            ),
        ];
        // fb vs a: conflict (fallback + conditional)
        // fb vs b: conflict (fallback + conditional)
        // a vs b: conflict (same model_id)
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 3);
    }

    // ---------------------------------------------------------------
    // ConflictReport display
    // ---------------------------------------------------------------

    #[test]
    fn conflict_report_display() {
        let report = ConflictReport {
            scenario_a: "a".to_string(),
            scenario_b: "b".to_string(),
            reason: "test reason".to_string(),
        };
        let display = format!("{}", report);
        assert!(display.contains("a"));
        assert!(display.contains("b"));
        assert!(display.contains("test reason"));
    }

    // ---------------------------------------------------------------
    // Request params: key present in only one
    // ---------------------------------------------------------------

    #[test]
    fn request_params_only_one_has_params_conflict() {
        let scenarios = vec![
            conditional(
                "a",
                MatchCondition {
                    request_params: Some(
                        vec![("stream".to_string(), serde_json::json!(true))]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            ),
            conditional("b", MatchCondition::default()),
        ];
        // B has no constraints at all, A requires stream=true.
        // A request with stream=true matches both => conflict!
        let conflicts = detect_conflicts(&scenarios);
        assert_eq!(conflicts.len(), 1);
    }
}
