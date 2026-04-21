use crate::permission::engine::{Defaults, Effect, Rule, RuleSet};

/// Builder for constructing [`RuleSet`] instances.
#[derive(Debug, Default)]
pub struct RuleSetBuilder {
    version: Option<String>,
    rules: Vec<Rule>,
    defaults: Defaults,
    template_includes: Vec<String>,
    agent_creators: std::collections::HashMap<String, String>,
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
        let version = self
            .version
            .ok_or(RuleSetBuilderError::MissingField("version"))?;

        Ok(RuleSet {
            version,
            rules: self.rules,
            defaults: self.defaults,
            template_includes: self.template_includes,
            agent_creators: self.agent_creators,
        })
    }

    /// Add a template include (loads the named template from templates/ directory).
    pub fn template_include(mut self, name: impl Into<String>) -> Self {
        self.template_includes.push(name.into());
        self
    }

    /// Register an agent creator mapping: agent_id -> creator_user_id.
    /// The creator automatically gets full-access to the agent.
    pub fn agent_creator(
        mut self,
        agent_id: impl Into<String>,
        creator_user_id: impl Into<String>,
    ) -> Self {
        self.agent_creators
            .insert(agent_id.into(), creator_user_id.into());
        self
    }
}

/// Errors that can occur during RuleSet construction.
#[derive(Debug, thiserror::Error)]
pub enum RuleSetBuilderError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}
