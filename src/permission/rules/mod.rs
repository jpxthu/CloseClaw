//! Permission rule helpers and builders
//!
//! Provides builder patterns and validation for [`Rule`], [`Subject`], and [`RuleSet`] types.

use crate::permission::engine::{Action, Effect, MatchType, Rule, RuleSet, Subject, Defaults};

/// Builder for constructing [`Rule`] instances fluently.
#[derive(Debug, Default)]
pub struct RuleBuilder {
    name: Option<String>,
    subject: Option<Subject>,
    effect: Effect,
    actions: Vec<Action>,
}

impl RuleBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the rule name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the subject (who this rule applies to).
    pub fn subject(mut self, subject: Subject) -> Self {
        self.subject = Some(subject);
        self
    }

    /// Set the subject using a simple agent ID (exact match).
    pub fn subject_agent(mut self, agent: impl Into<String>) -> Self {
        self.subject = Some(Subject {
            agent: agent.into(),
            match_type: MatchType::Exact,
        });
        self
    }

    /// Set the subject with glob matching.
    pub fn subject_glob(mut self, agent: impl Into<String>) -> Self {
        self.subject = Some(Subject {
            agent: agent.into(),
            match_type: MatchType::Glob,
        });
        self
    }

    /// Set the effect to Allow.
    pub fn allow(mut self) -> Self {
        self.effect = Effect::Allow;
        self
    }

    /// Set the effect to Deny.
    pub fn deny(mut self) -> Self {
        self.effect = Effect::Deny;
        self
    }

    /// Add an action to this rule.
    pub fn action(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    /// Add multiple actions to this rule.
    pub fn actions(mut self, actions: impl IntoIterator<Item = Action>) -> Self {
        self.actions.extend(actions);
        self
    }

    /// Finalize and return the constructed [`Rule`].
    pub fn build(self) -> Result<Rule, RuleBuilderError> {
        let name = self.name.ok_or(RuleBuilderError::MissingField("name"))?;
        let subject = self.subject.ok_or(RuleBuilderError::MissingField("subject"))?;

        Ok(Rule {
            name,
            subject,
            effect: self.effect,
            actions: self.actions,
        })
    }
}

/// Errors that can occur during Rule construction.
#[derive(Debug, thiserror::Error)]
pub enum RuleBuilderError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

/// Builder for constructing [`RuleSet`] instances.
#[derive(Debug, Default)]
pub struct RuleSetBuilder {
    version: Option<String>,
    rules: Vec<Rule>,
    defaults: Defaults,
}

impl RuleSetBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the version string.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Add a single rule.
    pub fn rule(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Add multiple rules.
    pub fn rules(mut self, rules: impl IntoIterator<Item = Rule>) -> Self {
        self.rules.extend(rules);
        self
    }

    /// Set the defaults.
    pub fn defaults(mut self, defaults: Defaults) -> Self {
        self.defaults = defaults;
        self
    }

    /// Set a specific default effect for file operations.
    pub fn default_file(mut self, effect: Effect) -> Self {
        self.defaults.file = effect;
        self
    }

    /// Set a specific default effect for command operations.
    pub fn default_command(mut self, effect: Effect) -> Self {
        self.defaults.command = effect;
        self
    }

    /// Set a specific default effect for network operations.
    pub fn default_network(mut self, effect: Effect) -> Self {
        self.defaults.network = effect;
        self
    }

    /// Set a specific default effect for inter-agent operations.
    pub fn default_inter_agent(mut self, effect: Effect) -> Self {
        self.defaults.inter_agent = effect;
        self
    }

    /// Set a specific default effect for config write operations.
    pub fn default_config(mut self, effect: Effect) -> Self {
        self.defaults.config = effect;
        self
    }

    /// Finalize and return the constructed [`RuleSet`].
    pub fn build(self) -> Result<RuleSet, RuleSetBuilderError> {
        let version = self.version.ok_or(RuleSetBuilderError::MissingField("version"))?;

        Ok(RuleSet {
            version,
            rules: self.rules,
            defaults: self.defaults,
        })
    }
}

/// Errors that can occur during RuleSet construction.
#[derive(Debug, thiserror::Error)]
pub enum RuleSetBuilderError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

/// Validation helpers for rules and rulesets.
pub mod validation {
    use crate::permission::engine::{Rule, RuleSet};

    /// Validate a single rule.
    pub fn validate_rule(rule: &Rule) -> Vec<RuleValidationError> {
        let mut errors = Vec::new();

        if rule.name.is_empty() {
            errors.push(RuleValidationError::EmptyName);
        }

        if rule.subject.agent.is_empty() {
            errors.push(RuleValidationError::EmptySubjectAgent);
        }

        if rule.actions.is_empty() {
            errors.push(RuleValidationError::NoActions);
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
                errors.push(RuleSetValidationError::InvalidRule { index: idx, error: err });
            }
        }

        errors
    }

    /// Check if a ruleset has any deny rules (high priority).
    pub fn has_deny_rules(ruleset: &RuleSet) -> bool {
        ruleset.rules.iter().any(|r| r.effect == crate::permission::engine::Effect::Deny)
    }

    /// Check if a ruleset has any allow rules.
    pub fn has_allow_rules(ruleset: &RuleSet) -> bool {
        ruleset.rules.iter().any(|r| r.effect == crate::permission::engine::Effect::Allow)
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
    }

    /// A validation error for a RuleSet.
    #[derive(Debug, thiserror::Error)]
    pub enum RuleSetValidationError {
        #[error("ruleset version cannot be empty")]
        EmptyVersion,
        #[error("rule at index {index} is invalid: {error}")]
        InvalidRule { index: usize, error: RuleValidationError },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::actions::ActionBuilder;

    #[test]
    fn test_rule_builder() {
        let rule = RuleBuilder::new()
            .name("allow-read-home")
            .subject_agent("dev-agent-01")
            .allow()
            .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
            .build()
            .unwrap();

        assert_eq!(rule.name, "allow-read-home");
        assert_eq!(rule.subject.agent, "dev-agent-01");
        assert!(matches!(rule.effect, Effect::Allow));
        assert_eq!(rule.actions.len(), 1);
    }

    #[test]
    fn test_rule_builder_missing_name() {
        let result = RuleBuilder::new()
            .subject_agent("dev-agent-01")
            .build();

        assert!(matches!(result, Err(RuleBuilderError::MissingField("name"))));
    }

    #[test]
    fn test_ruleset_builder() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("test-rule")
                    .subject_agent("test-agent")
                    .allow()
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();

        assert_eq!(ruleset.version, "1.0");
        assert_eq!(ruleset.rules.len(), 1);
        assert_eq!(ruleset.defaults.file, Effect::Deny);
    }

    #[test]
    fn test_validation() {
        let empty_rule = Rule {
            name: String::new(),
            subject: Subject { agent: String::new(), match_type: MatchType::Exact },
            effect: Effect::Allow,
            actions: vec![],
        };

        let errors = validation::validate_rule(&empty_rule);
        assert_eq!(errors.len(), 3);
    }
}
