//! Defensive validation for workflow definitions.
//!
//! Validates a fully deserialized [`Workflow`] to ensure structural integrity
//! before it enters the engine runtime. This is called after serde
//! deserialization succeeds in [`Workflow::parse_frontmatter`].

use std::collections::HashMap;
use std::collections::HashSet;

use serde_yaml::Value;

use crate::definition::{JumpQuestion, Step, Transition, Workflow};
use crate::error::WorkflowError;

/// Validate a [`Workflow`] definition against all structural rules.
///
/// Rules cover: step id continuity, transition structure, jump question
/// constraints, and enum option validity. Error messages include step
/// positions for easy debugging.
pub(crate) fn validate_definition(wf: &Workflow) -> Result<(), WorkflowError> {
    // Rule 1: steps must be non-empty
    if wf.steps.is_empty() {
        return Err(WorkflowError::invalid_definition("steps must not be empty"));
    }

    for (i, step) in wf.steps.iter().enumerate() {
        validate_step(step, i, &wf.steps)?;
    }

    Ok(())
}

fn validate_step(step: &Step, index: usize, all_steps: &[Step]) -> Result<(), WorkflowError> {
    let ctx = |msg: String| format!("step {index}: {msg}");

    // Rule 2: step id must equal its index (0-based, consecutive)
    if step.id != index {
        return Err(WorkflowError::invalid_definition(ctx(format!(
            "expected id {index}, got {}",
            step.id
        ))));
    }

    validate_step_content(step, &ctx)?;

    // Rule 10: jump ids must be unique within the step
    validate_jump_ids_unique(step, &ctx)?;

    // Rules 11, 12: each jump question is valid
    for jump in &step.jump {
        validate_jump(jump, &ctx)?;
    }

    // Rule 5: transitions structure
    validate_transitions_structure(step, &ctx)?;

    // Rules 6, 7, 8, 9: each transition is valid
    validate_transitions(step, all_steps, &ctx)?;

    Ok(())
}

/// Rule 3 + Rule 4: step content completeness.
fn validate_step_content(step: &Step, ctx: &dyn Fn(String) -> String) -> Result<(), WorkflowError> {
    // Rule 3: name and goal must be non-empty (after trim)
    if step.name.trim().is_empty() {
        return Err(WorkflowError::invalid_definition(ctx(
            "name must not be empty".into(),
        )));
    }
    if step.goal.trim().is_empty() {
        return Err(WorkflowError::invalid_definition(ctx(
            "goal must not be empty".into(),
        )));
    }

    // Rule 4: verify must be non-empty and contain non-blank strings
    if step.verify.is_empty() {
        return Err(WorkflowError::invalid_definition(ctx(
            "verify checklist must not be empty".into(),
        )));
    }
    for (vi, item) in step.verify.iter().enumerate() {
        if item.trim().is_empty() {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "verify item {vi} must not be blank"
            ))));
        }
    }

    Ok(())
}

fn validate_jump_ids_unique(
    step: &Step,
    ctx: &dyn Fn(String) -> String,
) -> Result<(), WorkflowError> {
    let mut seen = HashMap::new();
    for jump in &step.jump {
        if let Some(prev) = seen.get(&jump.id) {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "duplicate jump id '{id}' (first at position {prev})",
                id = jump.id
            ))));
        }
        seen.insert(jump.id.clone(), seen.len());
    }
    Ok(())
}

fn validate_jump(jump: &JumpQuestion, ctx: &dyn Fn(String) -> String) -> Result<(), WorkflowError> {
    // Rule 11: question_type must be boolean or enum
    match jump.question_type.as_str() {
        "boolean" | "enum" => {}
        other => {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "jump '{}': unsupported question_type '{}' \
                 (must be 'boolean' or 'enum')",
                jump.id, other
            ))));
        }
    }

    // Rule 12: enum-specific option constraints
    if jump.question_type == "enum" {
        validate_enum_options(jump, ctx)?;
    }

    Ok(())
}

/// Rule 12: enum options must be non-empty, unique, ≤ 26,
/// and option_labels length must match options if provided.
fn validate_enum_options(
    jump: &JumpQuestion,
    ctx: &dyn Fn(String) -> String,
) -> Result<(), WorkflowError> {
    if jump.options.is_empty() {
        return Err(WorkflowError::invalid_definition(ctx(format!(
            "jump '{}': enum options must not be empty",
            jump.id
        ))));
    }
    // No duplicate options
    let mut seen = HashSet::new();
    for opt in &jump.options {
        if !seen.insert(opt) {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "jump '{}': duplicate option '{}'",
                jump.id, opt
            ))));
        }
    }
    if jump.options.len() > 26 {
        return Err(WorkflowError::invalid_definition(ctx(format!(
            "jump '{}': enum options must not exceed 26 (got {})",
            jump.id,
            jump.options.len()
        ))));
    }
    // option_labels length must match options if provided
    if !jump.option_labels.is_empty() && jump.option_labels.len() != jump.options.len() {
        return Err(WorkflowError::invalid_definition(ctx(format!(
            "jump '{}': option_labels length ({}) must match \
             options length ({})",
            jump.id,
            jump.option_labels.len(),
            jump.options.len()
        ))));
    }
    Ok(())
}

fn validate_transitions_structure(
    step: &Step,
    ctx: &dyn Fn(String) -> String,
) -> Result<(), WorkflowError> {
    // Rule 5: transitions must not be empty
    if step.transitions.is_empty() {
        return Err(WorkflowError::invalid_definition(ctx(
            "transitions must not be empty".into(),
        )));
    }

    // Rule 5 cont: last transition must be default (no `when`)
    let last = step.transitions.last().unwrap();
    if last.when.is_some() {
        return Err(WorkflowError::invalid_definition(ctx(
            "last transition must be the default (no 'when')".into(),
        )));
    }

    // Rule 5 cont: all non-last transitions must have `when`
    let last_idx = step.transitions.len() - 1;
    for (ti, t) in step.transitions[..last_idx].iter().enumerate() {
        if t.when.is_none() {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "transition {ti} must have 'when' condition \
                 (only the last transition may be default)"
            ))));
        }
    }

    Ok(())
}

fn validate_transitions(
    step: &Step,
    all_steps: &[Step],
    ctx: &dyn Fn(String) -> String,
) -> Result<(), WorkflowError> {
    let steps_len = all_steps.len();
    let mut seen_when_keys: Vec<serde_yaml::Value> = Vec::new();

    for (ti, t) in step.transitions.iter().enumerate() {
        validate_transition_action(t, steps_len, ti, ctx)?;

        // Skip default (last) transition for when-related checks
        if t.when.is_none() {
            continue;
        }

        let when_val = t.when.as_ref().unwrap();

        // Rule 8: when must be a mapping with valid keys
        let mapping = validate_when_mapping(when_val, ti, step, ctx)?;

        // Rule 6: no duplicate when conditions across transitions
        let when_normalized = serialize_when(when_val);
        if seen_when_keys
            .iter()
            .any(|k| serialize_when(k) == when_normalized)
        {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "transition {ti}: duplicate 'when' condition"
            ))));
        }
        seen_when_keys.push(when_val.clone());

        // Rule 9: expected_value validation per jump type
        validate_expected_value(mapping, ti, step, ctx)?;
    }

    Ok(())
}

/// Rule 7: validate transition action type and target_step.
fn validate_transition_action(
    t: &Transition,
    steps_len: usize,
    ti: usize,
    ctx: &dyn Fn(String) -> String,
) -> Result<(), WorkflowError> {
    match t.action.as_str() {
        "goto" | "reexecute" => match t.target_step {
            Some(target) if target < steps_len => {}
            _ => {
                return Err(WorkflowError::invalid_definition(ctx(format!(
                    "transition {ti} ({}): \
                         target_step is invalid or out of range",
                    t.action
                ))));
            }
        },
        "complete" => {} // complete has no target
        other => {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "transition {ti}: unknown action '{}' \
                 (must be 'goto', 'reexecute', or 'complete')",
                other
            ))));
        }
    }
    Ok(())
}

/// Rule 8: validate that `when` is a mapping whose keys reference
/// defined jump ids and are unique within the mapping.
fn validate_when_mapping<'a>(
    when_val: &'a Value,
    ti: usize,
    step: &Step,
    ctx: &dyn Fn(String) -> String,
) -> Result<&'a serde_yaml::Mapping, WorkflowError> {
    let mapping = match when_val.as_mapping() {
        Some(m) => m,
        None => {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "transition {ti}: 'when' must be a mapping, \
                 got {when_val:?}"
            ))));
        }
    };

    let jump_ids: HashSet<_> = step.jump.iter().map(|j| j.id.as_str()).collect();

    // Keys must reference defined jump ids
    for key in mapping.keys() {
        let key_str = match key.as_str() {
            Some(s) => s,
            None => {
                return Err(WorkflowError::invalid_definition(ctx(format!(
                    "transition {ti}: 'when' key must be a string"
                ))));
            }
        };
        if !jump_ids.contains(key_str) {
            return Err(WorkflowError::invalid_definition(ctx(format!(
                "transition {ti}: 'when' key '{key_str}' \
                 does not match any jump id in this step"
            ))));
        }
    }

    // Note: serde_yaml::Mapping guarantees unique keys, so no
    // additional duplicate-key check is needed here.

    Ok(mapping)
}

/// Rule 9: validate expected_value matches the jump question type.
fn validate_expected_value(
    mapping: &serde_yaml::Mapping,
    ti: usize,
    step: &Step,
    ctx: &dyn Fn(String) -> String,
) -> Result<(), WorkflowError> {
    for key in mapping.keys() {
        let key_str = key.as_str().unwrap();
        let expected = mapping.get(key).unwrap();

        if let Some(jump) = step.jump.iter().find(|j| j.id == key_str) {
            match jump.question_type.as_str() {
                "boolean" => {
                    if !expected.is_bool() {
                        return Err(WorkflowError::invalid_definition(ctx(format!(
                            "transition {ti}: jump '{}' is boolean \
                                 but expected_value is not a bool",
                            key_str
                        ))));
                    }
                }
                "enum" => {
                    if let Some(val_str) = expected.as_str() {
                        if !jump.options.iter().any(|o| o == val_str) {
                            return Err(WorkflowError::invalid_definition(ctx(format!(
                                "transition {ti}: \
                                         expected_value '{}' is not \
                                         in enum options for jump \
                                         '{}'",
                                val_str, key_str
                            ))));
                        }
                    } else {
                        return Err(WorkflowError::invalid_definition(ctx(format!(
                            "transition {ti}: jump '{}' is \
                                     enum but expected_value is not \
                                     a string",
                            key_str
                        ))));
                    }
                }
                other => {
                    return Err(WorkflowError::invalid_definition(ctx(format!(
                        "transition {ti}: jump '{}' has unsupported \
                         question_type '{other}' (must be \
                         'boolean' or 'enum')",
                        key_str
                    ))));
                }
            }
        }
    }
    Ok(())
}

/// Serialize a when value to a deterministic string for comparison.
fn serialize_when(val: &Value) -> String {
    serde_yaml::to_string(val).expect("Value is always serializable")
}
