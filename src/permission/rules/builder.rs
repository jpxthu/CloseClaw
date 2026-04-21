use crate::permission::engine::{Action, Effect, MatchType, Rule, Subject, TemplateRef};

/// Builder for constructing [`Rule`] instances fluently.
#[derive(Debug, Default)]
pub struct RuleBuilder {
    name: Option<String>,
    subject: Option<Subject>,
    effect: Effect,
    actions: Vec<Action>,
    template: Option<TemplateRef>,
    priority: i32,
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
        self.subject = Some(Subject::AgentOnly {
            agent: agent.into(),
            match_type: MatchType::Exact,
        });
        self
    }

    /// Set the subject with glob matching.
    pub fn subject_glob(mut self, agent: impl Into<String>) -> Self {
        self.subject = Some(Subject::AgentOnly {
            agent: agent.into(),
            match_type: MatchType::Glob,
        });
        self
    }

    /// Set the subject as UserAndAgent dual-key matching.
    pub fn subject_user_and_agent(
        mut self,
        user_id: impl Into<String>,
        agent: impl Into<String>,
        user_match: MatchType,
        agent_match: MatchType,
    ) -> Self {
        self.subject = Some(Subject::UserAndAgent {
            user_id: user_id.into(),
            agent: agent.into(),
            user_match,
            agent_match,
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

    /// Set a template reference for this rule.
    pub fn template(mut self, name: impl Into<String>) -> Self {
        self.template = Some(TemplateRef {
            name: name.into(),
            overrides: Default::default(),
        });
        self
    }

    /// Set the evaluation priority. Higher = evaluated first.
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Finalize and return the constructed [`Rule`].
    pub fn build(self) -> Result<Rule, RuleBuilderError> {
        let name = self.name.ok_or(RuleBuilderError::MissingField("name"))?;
        let subject = self
            .subject
            .ok_or(RuleBuilderError::MissingField("subject"))?;

        Ok(Rule {
            name,
            subject,
            effect: self.effect,
            actions: self.actions,
            template: self.template,
            priority: self.priority,
        })
    }
}

/// Errors that can occur during Rule construction.
#[derive(Debug, thiserror::Error)]
pub enum RuleBuilderError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}
