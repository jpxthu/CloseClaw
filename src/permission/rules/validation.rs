//! Validation helpers for rules and rulesets.

use crate::permission::engine::{Effect, Rule, RuleSet};

/// Validate a single rule.
pub fn validate_rule(rule: &Rule) -> Vec<RuleValidationError> {
    let mut errors = Vec::new();

    if rule.name.is_empty() {
        errors.push(RuleValidationError::EmptyName);
    }

    if rule.subject.agent_id().is_empty() {
        errors.push(RuleValidationError::EmptySubjectAgent);
    }

    let has_actions = !rule.actions.is_empty();
    let has_template = rule.template.is_some();
    if !has_actions && !has_template {
        errors.push(RuleValidationError::NoActions);
    }
    if has_actions && has_template {
        errors.push(RuleValidationError::ActionsAndTemplateMutuallyExclusive);
    }

    errors
}

/// Validate an entire RuleSet.
pub fn validate_ruleset(ruleset: &RuleSet) -> Vec<RuleSetValidationError> {
    let mut errors = Vec::new();

    if ruleset.version.is_empty() {
        errors.push(RuleSetValidationError::EmptyVersion);
    }

    for (idx, rule) in ruleset.rules.iter().enumerate() {
        let rule_errors = validate_rule(rule);
        for err in rule_errors {
            errors.push(RuleSetValidationError::InvalidRule {
                index: idx,
                error: err,
            });
        }
    }

    errors
}

/// Check if a ruleset has any deny rules (high priority).
pub fn has_deny_rules(ruleset: &RuleSet) -> bool {
    ruleset.rules.iter().any(|r| r.effect == Effect::Deny)
}

/// Check if a ruleset has any allow rules.
pub fn has_allow_rules(ruleset: &RuleSet) -> bool {
    ruleset.rules.iter().any(|r| r.effect == Effect::Allow)
}

/// A validation error for a single rule.
#[derive(Debug, thiserror::Error)]
pub enum RuleValidationError {
    #[error("rule name cannot be empty")]
    EmptyName,
    #[error("rule subject agent cannot be empty")]
    EmptySubjectAgent,
    #[error("rule must have at least one action")]
    NoActions,
    #[error("rule cannot have both 'actions' and 'template' (mutually exclusive)")]
    ActionsAndTemplateMutuallyExclusive,
}

/// A validation error for a RuleSet.
#[derive(Debug, thiserror::Error)]
pub enum RuleSetValidationError {
    #[error("ruleset version cannot be empty")]
    EmptyVersion,
    #[error("rule at index {index} is invalid: {error}")]
    InvalidRule {
        index: usize,
        error: RuleValidationError,
    },
}
