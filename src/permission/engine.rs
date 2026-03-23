//! Permission Engine - Core security component
//!
//! Runs as a separate OS process, evaluates access rules for agents.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;

/// RuleSet parsed from permissions.json
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleSet {
    pub version: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub defaults: Defaults,
    /// Names of templates to load from the templates/ directory.
    #[serde(default)]
    pub template_includes: Vec<String>,
    /// Agent creator mapping: agent_id -> creator_user_id.
    /// Used to automatically generate creator full-access rules.
    #[serde(default)]
    pub agent_creators: HashMap<String, String>,
}

/// Default permissions for each action type
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Defaults {
    #[serde(default = "default_deny")]
    pub file: Effect,
    #[serde(default = "default_deny")]
    pub command: Effect,
    #[serde(default = "default_deny")]
    pub network: Effect,
    #[serde(default = "default_deny")]
    pub inter_agent: Effect,
    #[serde(default = "default_deny")]
    pub config: Effect,
}

fn default_deny() -> Effect {
    Effect::Deny
}

/// A single permission rule
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub name: String,
    pub subject: Subject,
    pub effect: Effect,
    /// Actions explicitly defined on this rule.
    /// Mutually exclusive with `template`: at least one must be present.
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Template reference for template composition.
    /// Mutually exclusive with `actions`: at least one must be present.
    #[serde(default)]
    pub template: Option<TemplateRef>,
    /// Optional priority for evaluation ordering.
    /// Higher number = evaluated first. Default = 0.
    #[serde(default)]
    pub priority: i32,
}

impl Rule {
    /// Validates the rule: `actions` and `template` are mutually exclusive,
    /// and at least one must be present. Returns Err with the violation message.
    pub fn validate(&self) -> Result<(), String> {
        let has_actions = !self.actions.is_empty();
        let has_template = self.template.is_some();
        if has_actions && has_template {
            return Err(format!(
                "rule '{}': 'actions' and 'template' are mutually exclusive",
                self.name
            ));
        }
        if !has_actions && !has_template {
            return Err(format!(
                "rule '{}': at least one of 'actions' or 'template' must be present",
                self.name
            ));
        }
        Ok(())
    }
}

/// Reference to a template, optionally with parameter overrides.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TemplateRef {
    /// Name of the template to inherit from.
    pub name: String,
    /// Optional field-level overrides.
    /// Supported override keys: "effect", "actions", "agent".
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
}

/// Subject that a rule applies to.
///
/// Supports two matching modes:
/// - `AgentOnly` (match_mode = "agent_only" or absent): legacy mode, matches only by `agent` field
/// - `UserAndAgent` (match_mode = "user_and_agent"): dual-key match, both `user_id` AND `agent` must match
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "match_mode", rename_all = "snake_case", content = "fields")]
pub enum Subject {
    /// Legacy agent-only matching (backward compatible).
    AgentOnly {
        agent: String,
        #[serde(default)]
        match_type: MatchType,
    },
    /// Dual-key matching: both user_id AND agent must match.
    UserAndAgent {
        user_id: String,
        agent: String,
        #[serde(default)]
        user_match: MatchType,
        #[serde(default)]
        agent_match: MatchType,
    },
}

impl Subject {
    /// Returns the agent portion for index building and lookup.
    pub fn agent_id(&self) -> &str {
        match self {
            Subject::AgentOnly { agent, .. } => agent,
            Subject::UserAndAgent { agent, .. } => agent,
        }
    }

    /// Returns the user_id portion (empty string for AgentOnly).
    pub fn user_id(&self) -> &str {
        match self {
            Subject::AgentOnly { .. } => "",
            Subject::UserAndAgent { user_id, .. } => user_id,
        }
    }

    /// Returns true if this is an AgentOnly subject.
    pub fn is_agent_only(&self) -> bool {
        matches!(self, Subject::AgentOnly { .. })
    }

    /// Check if this subject matches the given caller.
    pub fn matches(&self, caller: &Caller) -> bool {
        match self {
            Subject::AgentOnly { agent, match_type } => {
                match match_type {
                    MatchType::Exact => agent == &caller.agent,
                    MatchType::Glob => glob_match(agent, &caller.agent),
                }
            }
            Subject::UserAndAgent { user_id, agent, user_match, agent_match } => {
                let user_ok = match user_match {
                    MatchType::Exact => user_id == &caller.user_id,
                    MatchType::Glob => glob_match(user_id, &caller.user_id),
                };
                let agent_ok = match agent_match {
                    MatchType::Exact => agent == &caller.agent,
                    MatchType::Glob => glob_match(agent, &caller.agent),
                };
                user_ok && agent_ok
            }
        }
    }
}

/// Custom deserializer for Subject that handles both old flat format and new tagged format.
mod subject_de {
    use super::*;

    impl<'de> Deserialize<'de> for Subject {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            // First, deserialize into a JSON Value to peek at the structure
            let json: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;

            // Check for match_mode discriminant
            let match_mode = json.get("match_mode")
                .and_then(|v| v.as_str())
                .map(String::from);

            match match_mode.as_deref() {
                Some("user_and_agent") => {
                    #[derive(serde::Deserialize)]
                    struct UAFields {
                        #[serde(alias = "user_id")]
                        user_id: String,
                        #[serde(alias = "agent")]
                        agent: String,
                        #[serde(alias = "user_match", default)]
                        user_match: MatchType,
                        #[serde(alias = "agent_match", default)]
                        agent_match: MatchType,
                    }
                    let fields: UAFields = serde_json::from_value(json)
                        .map_err(|e| serde::de::Error::custom(e))?;
                    Ok(Subject::UserAndAgent {
                        user_id: fields.user_id,
                        agent: fields.agent,
                        user_match: fields.user_match,
                        agent_match: fields.agent_match,
                    })
                }
                _ => {
                    #[derive(serde::Deserialize)]
                    struct AOFields {
                        #[serde(alias = "agent")]
                        agent: String,
                        #[serde(alias = "match", alias = "match_type", default)]
                        match_type: Option<MatchType>,
                    }
                    let fields: AOFields = serde_json::from_value(json)
                        .map_err(|e| serde::de::Error::custom(e))?;
                    Ok(Subject::AgentOnly {
                        agent: fields.agent,
                        match_type: fields.match_type.unwrap_or_default(),
                    })
                }
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    #[default]
    Exact,
    Glob,
}

/// An action that a rule permits or denies
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    File {
        operation: String,
        paths: Vec<String>,
    },
    Command {
        command: String,
        #[serde(default)]
        args: CommandArgs,
    },
    Network {
        #[serde(default)]
        hosts: Vec<String>,
        #[serde(default)]
        ports: Vec<u16>,
    },
    ToolCall {
        skill: String,
        #[serde(default)]
        methods: Vec<String>,
    },
    InterAgent {
        #[serde(default)]
        agents: Vec<String>,
    },
    ConfigWrite {
        #[serde(default)]
        files: Vec<String>,
    },
    /// Matches any permission request. Used for admin/operator full-access rules.
    All,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum CommandArgs {
    #[default]
    Any,
    Allowed { allowed: Vec<String> },
    Blocked { blocked: Vec<String> },
}

/// Permission effect
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    #[default]
    Deny,
    Allow,
}

// ---------------------------------------------------------------------------
// PermissionRequest envelope (WithCaller / Bare)
// ---------------------------------------------------------------------------

/// Metadata about who/what initiated a permission request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Caller {
    /// The user ID of the message source (e.g. Feishu open_id, ou_xxx).
    /// Empty means "system caller" or "backward-compatible bare request".
    #[serde(default)]
    pub user_id: String,
    /// The agent instance ID (always present).
    pub agent: String,
    /// The user ID of the agent's creator (for creator-rule generation).
    /// If empty, looked up from agent_creators map at evaluation time.
    #[serde(default)]
    pub creator_id: String,
}

/// The actual request body (mirrors the existing PermissionRequest variants).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionRequestBody {
    FileOp { agent: String, path: String, op: String },
    CommandExec { agent: String, cmd: String, args: Vec<String> },
    NetOp { agent: String, host: String, port: u16 },
    ToolCall { agent: String, skill: String, method: String },
    InterAgentMsg { from: String, to: String },
    ConfigWrite { agent: String, config_file: String },
}

impl PermissionRequestBody {
    /// Extract agent ID from the request body.
    pub fn agent_id(&self) -> &str {
        match self {
            PermissionRequestBody::FileOp { agent, .. } => agent,
            PermissionRequestBody::CommandExec { agent, .. } => agent,
            PermissionRequestBody::NetOp { agent, .. } => agent,
            PermissionRequestBody::ToolCall { agent, .. } => agent,
            PermissionRequestBody::InterAgentMsg { from, .. } => from,
            PermissionRequestBody::ConfigWrite { agent, .. } => agent,
        }
    }
}

/// Permission request envelope — wraps the typed request with caller metadata.
///
/// For backward compatibility with existing callers that send bare
/// `PermissionRequest` variants, the engine also accepts bare requests
/// (without caller info) and treats them as:
///   - caller.user_id = ""
///   - caller.agent   = extracted from the request variant
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PermissionRequest {
    /// Full request with caller metadata (new format).
    WithCaller {
        caller: Caller,
        #[serde(flatten)]
        request: PermissionRequestBody,
    },
    /// Backward-compatible bare request without caller info.
    Bare(PermissionRequestBody),
}

impl PermissionRequest {
    /// Returns the caller metadata, with empty defaults for Bare requests.
    pub fn caller(&self) -> Caller {
        match self {
            PermissionRequest::WithCaller { caller, .. } => caller.clone(),
            PermissionRequest::Bare(body) => {
                Caller {
                    user_id: String::new(),
                    agent: body.agent_id().to_string(),
                    creator_id: String::new(),
                }
            }
        }
    }

    /// Returns the agent ID from the request body.
    pub fn agent_id(&self) -> &str {
        match self {
            PermissionRequest::WithCaller { request, .. } => request.agent_id(),
            PermissionRequest::Bare(body) => body.agent_id(),
        }
    }

    /// Converts a bare request to a request with caller.
    pub fn with_caller(self, caller: Caller) -> PermissionRequest {
        match self {
            PermissionRequest::Bare(body) => PermissionRequest::WithCaller { caller, request: body },
            other @ PermissionRequest::WithCaller { .. } => other,
        }
    }

    /// Unwrap to the inner body if Bare, or extract from WithCaller.
    pub fn into_body(self) -> PermissionRequestBody {
        match self {
            PermissionRequest::WithCaller { request, .. } => request,
            PermissionRequest::Bare(body) => body,
        }
    }

    /// Access the inner body reference.
    pub fn body(&self) -> &PermissionRequestBody {
        match self {
            PermissionRequest::WithCaller { request, .. } => request,
            PermissionRequest::Bare(body) => body,
        }
    }
}

/// Permission response from the engine
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PermissionResponse {
    Allowed { token: String },
    Denied { reason: String, rule: String },
}

// ---------------------------------------------------------------------------
// PermissionEngine
// ---------------------------------------------------------------------------

/// Permission Engine - evaluates access requests against rules
pub struct PermissionEngine {
    /// RuleSet
    rules: RuleSet,
    /// O(1) lookup index: agent_id -> list of rule indices
    agent_rule_index: HashMap<String, Vec<usize>>,
    /// O(1) lookup index: "{user_id}:{agent_id}" -> list of rule indices
    user_agent_rule_index: HashMap<String, Vec<usize>>,
    /// Loaded templates: name -> Template
    templates: HashMap<String, crate::permission::templates::Template>,
}

impl PermissionEngine {
    /// Create a new PermissionEngine from a RuleSet
    pub fn new(rules: RuleSet) -> Self {
        let mut engine = Self {
            rules: rules.clone(),
            agent_rule_index: HashMap::new(),
            user_agent_rule_index: HashMap::new(),
            templates: HashMap::new(),
        };
        // Build indices from the rules (pass by value to avoid borrow conflict)
        let agent_index: HashMap<String, Vec<usize>> = HashMap::new();
        let user_agent_index: HashMap<String, Vec<usize>> = HashMap::new();
        engine.agent_rule_index = agent_index;
        engine.user_agent_rule_index = user_agent_index;
        for (idx, rule) in rules.rules.iter().enumerate() {
            match &rule.subject {
                Subject::AgentOnly { agent, .. } => {
                    engine.agent_rule_index.entry(agent.clone()).or_default().push(idx);
                }
                Subject::UserAndAgent { user_id, agent, .. } => {
                    let key = format!("{}:{}", user_id, agent);
                    engine.user_agent_rule_index.entry(key).or_default().push(idx);
                    engine.agent_rule_index.entry(agent.clone()).or_default().push(idx);
                }
            }
        }
        engine
    }

    /// Rebuild the lookup indices from a given ruleset (sync helper).
    fn rebuild_indices_with_rules(&mut self, rules: &RuleSet) {
        let mut agent_index: HashMap<String, Vec<usize>> = HashMap::new();
        let mut user_agent_index: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, rule) in rules.rules.iter().enumerate() {
            match &rule.subject {
                Subject::AgentOnly { agent, .. } => {
                    agent_index.entry(agent.clone()).or_default().push(idx);
                }
                Subject::UserAndAgent { user_id, agent, .. } => {
                    let key = format!("{}:{}", user_id, agent);
                    user_agent_index.entry(key).or_default().push(idx);
                    // Also index by agent alone for backward-compatible glob scan
                    agent_index.entry(agent.clone()).or_default().push(idx);
                }
            }
        }

        self.agent_rule_index = agent_index;
        self.user_agent_rule_index = user_agent_index;
    }

    /// Reload rules from a new RuleSet
    pub fn reload_rules(&mut self, rules: RuleSet) {
        self.rebuild_indices_with_rules(&rules);
        self.rules = rules;
    }

    /// Load templates into the engine
    pub fn load_templates(&mut self, templates: HashMap<String, crate::permission::templates::Template>) {
        self.templates = templates;
    }

    /// Simplified permission check — evaluates if `agent_id` may perform `action`.
    ///
    /// `action` is one of: "exec", "file_read", "file_write", "network",
    /// "spawn", "tool_call", "config_write".
    ///
    /// Uses a bare request (no caller user context) and wildcard arguments
    /// to check the coarse-grained permission.
    pub fn check(&self, agent_id: &str, action: &str) -> PermissionResponse {
        let body = match action {
            "exec" => PermissionRequestBody::CommandExec {
                agent: agent_id.to_string(),
                cmd: "*".to_string(),
                args: Vec::new(),
            },
            "file_read" => PermissionRequestBody::FileOp {
                agent: agent_id.to_string(),
                path: "*".to_string(),
                op: "read".to_string(),
            },
            "file_write" => PermissionRequestBody::FileOp {
                agent: agent_id.to_string(),
                path: "*".to_string(),
                op: "write".to_string(),
            },
            "network" => PermissionRequestBody::NetOp {
                agent: agent_id.to_string(),
                host: "*".to_string(),
                port: 0,
            },
            "spawn" => PermissionRequestBody::InterAgentMsg {
                from: agent_id.to_string(),
                to: "*".to_string(),
            },
            "tool_call" => PermissionRequestBody::ToolCall {
                agent: agent_id.to_string(),
                skill: "*".to_string(),
                method: "*".to_string(),
            },
            "config_write" => PermissionRequestBody::ConfigWrite {
                agent: agent_id.to_string(),
                config_file: "*".to_string(),
            },
            // Unknown action — deny by default
            _ => {
                return PermissionResponse::Denied {
                    reason: format!("unknown action: {}", action),
                    rule: "<check>".to_string(),
                };
            }
        };

        self.evaluate(PermissionRequest::Bare(body))
    }

    /// Evaluate a permission request
    pub fn evaluate(&self, request: PermissionRequest) -> PermissionResponse {
        let caller = request.caller();
        let agent_id = caller.agent.clone();

        info!(
            agent = %agent_id,
            user_id = %caller.user_id,
            request_type = ?request.body(),
            "permission check initiated"
        );

        // Clone rules to avoid holding reference across await
        let rules = self.rules.clone();

        // ---- Step 0: Creator Rule (highest priority, short-circuit return) ----
        let effective_creator_id = if !caller.creator_id.is_empty() {
            Some(caller.creator_id.as_str())
        } else {
            rules.agent_creators.get(&agent_id).map(|s| s.as_str())
        };
        if let Some(creator_id) = effective_creator_id {
            if caller.user_id == creator_id {
                info!(agent = %agent_id, result = "allowed", reason = "creator_rule", "permission check completed");
                return PermissionResponse::Allowed { token: generate_token() };
            }
        }

        // ---- Step 1: Build candidate rule list ----
        let mut candidates: Vec<usize> = Vec::new();

        // 1a. User+Agent dual-key index lookup (O(1))
        let index_key = format!("{}:{}", caller.user_id, agent_id);
        if let Some(indices) = self.user_agent_rule_index.get(&index_key) {
            candidates.extend(indices);
        }

        // 1b. Agent-only index lookup (O(1))
        if let Some(indices) = self.agent_rule_index.get(&agent_id) {
            candidates.extend(indices);
        }

        // 1c. Glob fallback (only if 1a and 1b produced nothing)
        if candidates.is_empty() {
            for (idx, rule) in rules.rules.iter().enumerate() {
                if rule.subject.matches(&caller) {
                    candidates.push(idx);
                }
            }
        }

        // ---- Step 2: Sort by priority (desc) ----
        candidates.sort_by(|&a, &b| {
            rules.rules[b]
                .priority
                .cmp(&rules.rules[a].priority)
        });

        // ---- Step 3: Expand templates ----
        // For template-based rules, expand them into pseudo-rules with resolved actions.
        // Returns (expanded_rules_flat, indices_into_flat).
        let (expanded_rules, expanded_indices) = self.expand_templates_sync(&candidates, &rules);

        // ---- Step 4: Evaluate (AWS IAM-style deny-precedence) ----
        // First pass: collect all matching (subject+action) rules
        let mut matching_rule_name: Option<String> = None;
        for &rule_idx in &expanded_indices {
            let rule = &expanded_rules[rule_idx];

            // Subject match
            if !rule.subject.matches(&caller) {
                continue;
            }

            // Action match (for template-expanded rules, actions are pre-resolved)
            if !self.rule_actions_match(rule, request.body()) {
                continue;
            }

            matching_rule_name = Some(rule.name.clone());

            // Deny wins immediately
            if rule.effect == Effect::Deny {
                let reason = format!("action denied by rule '{}'", rule.name);
                info!(agent = %agent_id, result = "denied", rule = %rule.name, "permission check completed");
                return PermissionResponse::Denied {
                    reason,
                    rule: rule.name.clone(),
                };
            }
        }

        // No deny found; if any rule matched, allow
        if matching_rule_name.is_some() {
            info!(agent = %agent_id, result = "allowed", reason = "matched_rule", "permission check completed");
            return PermissionResponse::Allowed { token: generate_token() };
        }

        // ---- Step 5: Default fallback ----
        let response = self.default_deny(request.body(), &rules.defaults, "no matching rule");
        info!(
            agent = %agent_id,
            result = %match &response {
                PermissionResponse::Allowed { .. } => "allowed",
                PermissionResponse::Denied { .. } => "denied",
            },
            reason = "default_fallback",
            "permission check completed"
        );
        response
    }

    /// Expand template references in candidate rules.
    ///
    /// For template-based rules, replaces the rule with one pseudo-rule per resolved
    /// template action (each pseudo-rule carries a single action, allowing precise
    /// deny/action matching). For non-template rules, keeps the rule as-is.
    ///
    /// Returns `(expanded_rules_flat, indices_into_flat)` where `indices_into_flat`
    /// are indices into `expanded_rules_flat`.
    fn expand_templates_sync(
        &self,
        candidates: &[usize],
        ruleset: &RuleSet,
    ) -> (Vec<Rule>, Vec<usize>) {
        let mut expanded_rules: Vec<Rule> = Vec::new();
        let mut expanded_indices: Vec<usize> = Vec::new();

        for &idx in candidates {
            let rule = &ruleset.rules[idx];

            if let Some(ref template_ref) = rule.template {
                // Template-based rule: expand into pseudo-rules, one per resolved action
                if let Some(tmpl) = self.templates.get(&template_ref.name) {
                    let actions =
                        resolve_template_actions(tmpl, &template_ref.overrides);
                    for action in actions {
                        // Create pseudo-rule with single resolved action
                        let pseudo_rule = Rule {
                            name: rule.name.clone(),
                            subject: rule.subject.clone(),
                            effect: rule.effect,
                            actions: vec![action],
                            template: None,
                            priority: rule.priority,
                        };
                        expanded_indices.push(expanded_rules.len());
                        expanded_rules.push(pseudo_rule);
                    }
                }
                // If template not found: skip (template resolution failed, rule won't match)
            } else {
                // Non-template rule: keep as-is
                expanded_indices.push(expanded_rules.len());
                expanded_rules.push(rule.clone());
            }
        }

        // Deduplicate while preserving order (by index into expanded_rules)
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut unique_indices: Vec<usize> = Vec::new();
        for &idx in &expanded_indices {
            if seen.insert(idx) {
                unique_indices.push(idx);
            }
        }

        (expanded_rules, unique_indices)
    }

    /// Get default action when no rule matches
    fn default_deny(
        &self,
        request: &PermissionRequestBody,
        defaults: &Defaults,
        reason: &str,
    ) -> PermissionResponse {
        let effect = match request {
            PermissionRequestBody::FileOp { .. } => defaults.file,
            PermissionRequestBody::CommandExec { .. } => defaults.command,
            PermissionRequestBody::NetOp { .. } => defaults.network,
            PermissionRequestBody::InterAgentMsg { .. } => defaults.inter_agent,
            PermissionRequestBody::ConfigWrite { .. } => defaults.config,
            PermissionRequestBody::ToolCall { .. } => defaults.file,
        };

        match effect {
            Effect::Allow => PermissionResponse::Allowed { token: generate_token() },
            Effect::Deny => PermissionResponse::Denied {
                reason: reason.to_string(),
                rule: "default".to_string(),
            },
        }
    }

    /// Check if a rule's actions match the request
    fn rule_actions_match(&self, rule: &Rule, request: &PermissionRequestBody) -> bool {
        // If rule has a template reference, resolve template actions
        let actions = if let Some(ref template_ref) = rule.template {
            if let Some(tmpl) = self.templates.get(&template_ref.name) {
                resolve_template_actions(tmpl, &template_ref.overrides)
            } else {
                rule.actions.clone()
            }
        } else {
            rule.actions.clone()
        };

        for action in &actions {
            if action_matches_request(action, request) {
                return true;
            }
        }
        false
    }

    /// Check if command arguments match
    /// For Allowed: returns true if ALL request args are in the allowed list
    /// For Blocked: returns true if ANY request arg is in the blocked list
    pub fn args_match(&self, rule_args: &CommandArgs, request_args: &[String]) -> bool {
        match rule_args {
            CommandArgs::Any => true,
            CommandArgs::Allowed { allowed } => {
                request_args.iter().all(|arg| allowed.iter().any(|a| glob_match(a, arg)))
            }
            CommandArgs::Blocked { blocked } => {
                request_args.iter().any(|arg| blocked.iter().any(|b| glob_match(b, arg)))
            }
        }
    }
}

/// Resolve template actions with overrides applied.
fn resolve_template_actions(
    tmpl: &crate::permission::templates::Template,
    overrides: &HashMap<String, serde_json::Value>,
) -> Vec<Action> {
    if let Some(overridden_actions) = overrides.get("actions") {
        if let Ok(actions) = serde_json::from_value(overridden_actions.clone()) {
            return actions;
        }
    }
    tmpl.actions.clone()
}

/// Check if a single action matches the request.
pub(crate) fn action_matches_request(action: &Action, request: &PermissionRequestBody) -> bool {
    if matches!(action, Action::All) {
        return true;
    }

    match (action, request) {
        (Action::File { operation, paths }, PermissionRequestBody::FileOp { path, op, .. }) => {
            operation == op && paths.iter().any(|p| glob_match(p, path))
        }
        (Action::Command { command, args }, PermissionRequestBody::CommandExec { cmd, args: req_args, .. }) => {
            if command != cmd {
                return false;
            }
            match args {
                CommandArgs::Any => true,
                CommandArgs::Allowed { allowed } => {
                    req_args.iter().all(|arg| allowed.iter().any(|a| glob_match(a, arg)))
                }
                CommandArgs::Blocked { blocked } => {
                    req_args.iter().any(|arg| blocked.iter().any(|b| glob_match(b, arg)))
                }
            }
        }
        (Action::Network { hosts, ports }, PermissionRequestBody::NetOp { host, port, .. }) => {
            (hosts.is_empty() || hosts.iter().any(|h| glob_match(h, host)))
                && (ports.is_empty() || ports.contains(port))
        }
        (Action::ToolCall { skill, methods }, PermissionRequestBody::ToolCall { skill: s, method, .. }) => {
            skill == s && (methods.is_empty() || methods.contains(method))
        }
        (Action::InterAgent { agents }, PermissionRequestBody::InterAgentMsg { to, .. }) => {
            agents.is_empty() || agents.iter().any(|a| glob_match(a, to))
        }
        (Action::ConfigWrite { files }, PermissionRequestBody::ConfigWrite { config_file, .. }) => {
            files.is_empty() || files.iter().any(|f| glob_match(f, config_file))
        }
        _ => false,
    }
}

/// Simple glob matching (supports * and **)
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "**" || pattern == "*" {
        return true;
    }

    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    glob_match_vec(&pattern_chars, &text_chars, 0, 0)
}

fn glob_match_vec(pat: &[char], text: &[char], pi: usize, ti: usize) -> bool {
    if pi == pat.len() && ti == text.len() {
        return true;
    }

    if pi < pat.len() && pat[pi] == '*' {
        if pi + 1 < pat.len() && pat[pi + 1] == '*' {
            if pi + 2 < pat.len() && (pat[pi + 2] == '/' || pat[pi + 2] == '\\') {
                if pi + 3 < pat.len() {
                    return glob_match_vec(pat, text, pi + 3, ti)
                        || (ti < text.len() && glob_match_vec(pat, text, pi, ti + 1));
                }
                return ti >= text.len() || text[ti] == '/';
            }
            return ti >= text.len()
                || glob_match_vec(pat, text, pi + 2, ti)
                || glob_match_vec(pat, text, pi, ti + 1);
        }
        if ti >= text.len() {
            return glob_match_vec(pat, text, pi + 1, ti);
        }
        return text[ti] != '/'
            && (glob_match_vec(pat, text, pi + 1, ti) || glob_match_vec(pat, text, pi, ti + 1));
    }

    if pi < pat.len() && ti < text.len() && (pat[pi] == '?' || pat[pi] == text[ti]) {
        return glob_match_vec(pat, text, pi + 1, ti + 1);
    }

    false
}

/// Generate a short-lived permission token
fn generate_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    format!("perm_{}_{:016x}", duration.as_secs(), rand_u64())
}

fn rand_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish()
}

// ---------------------------------------------------------------------------
// Backward-compatible Rule parsing for Subject
// ---------------------------------------------------------------------------

impl Rule {
    /// Parse subject from string (for testing) — creates an AgentOnly subject.
    pub fn parse_subject(agent: &str) -> Subject {
        Subject::AgentOnly {
            agent: agent.to_string(),
            match_type: MatchType::Exact,
        }
    }

    /// Parse subject with match type (for testing).
    pub fn parse_subject_with_match(agent: &str, match_type: &str) -> Subject {
        Subject::AgentOnly {
            agent: agent.to_string(),
            match_type: match match_type {
                "glob" => MatchType::Glob,
                _ => MatchType::Exact,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Glob matching tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_glob_exact() {
        assert!(glob_match("dev-agent-01", "dev-agent-01"));
        assert!(!glob_match("dev-agent-01", "dev-agent-02"));
    }

    #[test]
    fn test_glob_star() {
        assert!(glob_match("readonly-*", "readonly-agent-1"));
        assert!(glob_match("readonly-*", "readonly-agent-42"));
        assert!(!glob_match("readonly-*", "readonly"));
    }

    #[test]
    fn test_glob_double_star() {
        assert!(glob_match("/home/admin/code/**", "/home/admin/code/closeclaw/src/main.rs"));
        assert!(glob_match("/home/admin/code/**", "/home/admin/code/closeclaw/src/permission/engine.rs"));
        assert!(!glob_match("/home/admin/code/**", "/home/admin/other/path"));
    }

    #[test]
    fn test_glob_question() {
        assert!(glob_match("file_?.txt", "file_a.txt"));
        assert!(glob_match("file_?.txt", "file_1.txt"));
        assert!(!glob_match("file_?.txt", "file_12.txt"));
    }

    // -------------------------------------------------------------------------
    // Subject matching tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_subject_agent_only_exact() {
        let subject = Subject::AgentOnly {
            agent: "dev-agent-01".to_string(),
            match_type: MatchType::Exact,
        };
        let caller = Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        };
        assert!(subject.matches(&caller));
        let caller = Caller {
            user_id: "ou_alice".to_string(),
            agent: "other-agent".to_string(),
            creator_id: String::new(),
        };
        assert!(!subject.matches(&caller));
    }

    #[test]
    fn test_subject_agent_only_glob() {
        let subject = Subject::AgentOnly {
            agent: "dev-*".to_string(),
            match_type: MatchType::Glob,
        };
        let caller = Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        };
        assert!(subject.matches(&caller));
    }

    #[test]
    fn test_subject_user_and_agent_both_match() {
        let subject = Subject::UserAndAgent {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        };
        let caller = Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        };
        assert!(subject.matches(&caller));
    }

    #[test]
    fn test_subject_user_and_agent_user_mismatch() {
        let subject = Subject::UserAndAgent {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        };
        let caller = Caller {
            user_id: "ou_bob".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        };
        assert!(!subject.matches(&caller));
    }

    #[test]
    fn test_subject_user_and_agent_agent_mismatch() {
        let subject = Subject::UserAndAgent {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        };
        let caller = Caller {
            user_id: "ou_alice".to_string(),
            agent: "other-agent".to_string(),
            creator_id: String::new(),
        };
        assert!(!subject.matches(&caller));
    }

    #[test]
    fn test_subject_user_and_agent_glob() {
        let subject = Subject::UserAndAgent {
            user_id: "ou_admin_*".to_string(),
            agent: "dev-*".to_string(),
            user_match: MatchType::Glob,
            agent_match: MatchType::Glob,
        };
        let caller = Caller {
            user_id: "ou_admin_john".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        };
        assert!(subject.matches(&caller));
    }

    #[test]
    fn test_subject_is_agent_only() {
        assert!(Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        }.is_agent_only());
        assert!(!Subject::UserAndAgent {
            user_id: "ou_123".to_string(),
            agent: "test".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        }.is_agent_only());
    }

    // -------------------------------------------------------------------------
    // Rule validation tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_rule_validate_actions_only() {
        let rule = Rule {
            name: "test".to_string(),
            subject: Subject::AgentOnly {
                agent: "test".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::File {
                operation: "read".to_string(),
                paths: vec!["**".to_string()],
            }],
            template: None,
            priority: 0,
        };
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn test_rule_validate_template_only() {
        let rule = Rule {
            name: "test".to_string(),
            subject: Subject::AgentOnly {
                agent: "test".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![],
            template: Some(TemplateRef {
                name: "developer".to_string(),
                overrides: HashMap::new(),
            }),
            priority: 0,
        };
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn test_rule_validate_mutually_exclusive() {
        let rule = Rule {
            name: "test".to_string(),
            subject: Subject::AgentOnly {
                agent: "test".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::File {
                operation: "read".to_string(),
                paths: vec!["**".to_string()],
            }],
            template: Some(TemplateRef {
                name: "developer".to_string(),
                overrides: HashMap::new(),
            }),
            priority: 0,
        };
        let err = rule.validate().unwrap_err();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn test_rule_validate_at_least_one() {
        let rule = Rule {
            name: "test".to_string(),
            subject: Subject::AgentOnly {
                agent: "test".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![],
            template: None,
            priority: 0,
        };
        let err = rule.validate().unwrap_err();
        assert!(err.contains("at least one"));
    }

    // -------------------------------------------------------------------------
    // PermissionRequest envelope tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_permission_request_bare_caller() {
        let body = PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/**".to_string(),
            op: "read".to_string(),
        };
        let request = PermissionRequest::Bare(body);
        let caller = request.caller();
        assert_eq!(caller.user_id, "");
        assert_eq!(caller.agent, "dev-agent-01");
    }

    #[test]
    fn test_permission_request_with_caller() {
        let request = PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "ou_alice".to_string(),
                agent: "dev-agent-01".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "dev-agent-01".to_string(),
                path: "/home/admin/code/**".to_string(),
                op: "read".to_string(),
            },
        };
        let caller = request.caller();
        assert_eq!(caller.user_id, "ou_alice");
        assert_eq!(caller.agent, "dev-agent-01");
    }

    #[test]
    fn test_permission_request_with_caller_deserialize() {
        let json = r#"{
            "caller": {"user_id": "ou_alice", "agent": "dev-agent-01"},
            "type": "file_op",
            "agent": "dev-agent-01",
            "path": "/home/admin/code/**",
            "op": "read"
        }"#;
        let request: PermissionRequest = serde_json::from_str(json).unwrap();
        let caller = request.caller();
        assert_eq!(caller.user_id, "ou_alice");
        assert_eq!(caller.agent, "dev-agent-01");
    }

    #[test]
    fn test_permission_request_bare_deserialize() {
        let json = r#"{
            "type": "file_op",
            "agent": "dev-agent-01",
            "path": "/home/admin/code/**",
            "op": "read"
        }"#;
        let request: PermissionRequest = serde_json::from_str(json).unwrap();
        let caller = request.caller();
        assert_eq!(caller.user_id, "");
        assert_eq!(caller.agent, "dev-agent-01");
    }

    #[test]
    fn test_permission_request_bare_into_with_caller() {
        let body = PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/**".to_string(),
            op: "read".to_string(),
        };
        let bare = PermissionRequest::Bare(body);
        let with_caller = bare.with_caller(Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        });
        let caller = with_caller.caller();
        assert_eq!(caller.user_id, "ou_alice");
    }

    #[test]
    fn test_permission_request_body_agent_id() {
        let body = PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/tmp".to_string(),
            op: "read".to_string(),
        };
        assert_eq!(body.agent_id(), "test-agent");
        let body = PermissionRequestBody::InterAgentMsg {
            from: "agent-a".to_string(),
            to: "agent-b".to_string(),
        };
        assert_eq!(body.agent_id(), "agent-a");
    }

    #[test]
    fn test_subject_deserialize_old_format() {
        // Old format: no match_mode field
        let json = r#"{"agent": "dev-agent-01", "match_type": "exact"}"#;
        let subject: Subject = serde_json::from_str(json).unwrap();
        assert!(matches!(subject, Subject::AgentOnly { .. }));
        assert_eq!(subject.agent_id(), "dev-agent-01");
    }

    #[test]
    fn test_subject_deserialize_old_format_glob() {
        let json = r#"{"agent": "dev-*", "match": "glob"}"#;
        let subject: Subject = serde_json::from_str(json).unwrap();
        assert!(matches!(subject, Subject::AgentOnly { agent, match_type: MatchType::Glob } if agent == "dev-*"));
    }

    #[test]
    fn test_subject_deserialize_new_agent_only() {
        let json = r#"{"match_mode": "agent_only", "agent": "dev-agent-01", "match_type": "exact"}"#;
        let subject: Subject = serde_json::from_str(json).unwrap();
        assert!(matches!(subject, Subject::AgentOnly { .. }));
    }

    #[test]
    fn test_subject_deserialize_new_user_and_agent() {
        let json = r#"{
            "match_mode": "user_and_agent",
            "user_id": "ou_alice",
            "agent": "dev-agent-01",
            "user_match": "exact",
            "agent_match": "exact"
        }"#;
        let subject: Subject = serde_json::from_str(json).unwrap();
        assert!(matches!(subject, Subject::UserAndAgent { .. }));
        let Subject::UserAndAgent { user_id, agent, .. } = subject else { unreachable!() };
        assert_eq!(user_id, "ou_alice");
        assert_eq!(agent, "dev-agent-01");
    }

    #[test]
    fn test_subject_deserialize_user_and_agent_glob() {
        let json = r#"{
            "match_mode": "user_and_agent",
            "user_id": "ou_admin_*",
            "agent": "dev-*",
            "user_match": "glob",
            "agent_match": "glob"
        }"#;
        let subject: Subject = serde_json::from_str(json).unwrap();
        assert!(matches!(subject, Subject::UserAndAgent { .. }));
    }

    #[test]
    fn test_subject_user_id() {
        let agent_only = Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        };
        assert_eq!(agent_only.user_id(), "");

        let user_agent = Subject::UserAndAgent {
            user_id: "ou_123".to_string(),
            agent: "test".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        };
        assert_eq!(user_agent.user_id(), "ou_123");
    }

    // -------------------------------------------------------------------------
    // Action::All tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_action_all_matches_request() {
        let action = Action::All;
        let body = PermissionRequestBody::FileOp {
            agent: "test".to_string(),
            path: "/any".to_string(),
            op: "any".to_string(),
        };
        assert!(action_matches_request(&action, &body));
        let body = PermissionRequestBody::CommandExec {
            agent: "test".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/".to_string()],
        };
        assert!(action_matches_request(&action, &body));
    }

    // -------------------------------------------------------------------------
    // Engine evaluate tests (backward compatibility)
    // -------------------------------------------------------------------------

    fn test_rules_json() -> &'static str {
        r#"{
  "version": "1.0",
  "rules": [
    {
      "name": "dev-agent-file-read",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "read",
          "paths": ["/home/admin/code/**"]
        }
      ]
    },
    {
      "name": "dev-agent-file-write",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "write",
          "paths": ["/home/admin/code/closeclaw/src/**"]
        }
      ]
    },
    {
      "name": "dev-agent-git",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "allowed": ["status", "log", "diff", "add", "commit", "push", "pull"] }
        }
      ]
    },
    {
      "name": "dev-agent-forbidden-git-reset",
      "subject": { "agent": "dev-agent-01" },
      "effect": "deny",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "blocked": ["reset", "rebase", "push", "--force"] }
        }
      ]
    },
    {
      "name": "readonly-agent",
      "subject": { "agent": "readonly-*", "match": "glob" },
      "effect": "allow",
      "actions": [
        { "type": "file", "operation": "read", "paths": ["**"] }
      ]
    }
  ],
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny",
    "config": "deny"
  }
}"#
    }

    #[tokio::test]
    async fn test_rule_parsing() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        assert_eq!(rules.version, "1.0");
        assert_eq!(rules.rules.len(), 5);
        assert_eq!(rules.defaults.file, Effect::Deny);
    }

    #[tokio::test]
    async fn test_file_read_allowed() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_file_read_denied_no_match() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/etc/passwd".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_file_write_allowed() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
            op: "write".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_command_allowed() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "dev-agent-01".to_string(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_command_denied_blocked() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "dev-agent-01".to_string(),
            cmd: "git".to_string(),
            args: vec!["reset".to_string(), "--hard".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_glob_matching() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "readonly-agent-42".to_string(),
            path: "/any/path/in/the/system.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_default_deny() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::Bare(PermissionRequestBody::NetOp {
            agent: "dev-agent-01".to_string(),
            host: "example.com".to_string(),
            port: 443,
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_unknown_agent_defaults_to_deny() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "unknown-agent".to_string(),
            path: "/home/admin/code/**".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_o1_lookup_performance() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
                agent: "dev-agent-01".to_string(),
                path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
                op: "read".to_string(),
            });
            let _ = engine.evaluate(request);
        }
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 100, "O(1) lookup should be fast, took {:?}", elapsed);
    }

    #[tokio::test]
    async fn test_permission_engine_parse() {
        let json = r#"{
            "version": "1.0",
            "rules": [],
            "defaults": { "effect": "deny" }
        }"#;
        let _rules: RuleSet = serde_json::from_str(json).unwrap();
    }

    // -------------------------------------------------------------------------
    // Comprehensive glob_match corner case tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_glob_match_exact_match() {
        assert!(glob_match("dev-agent-01", "dev-agent-01"));
        assert!(!glob_match("dev-agent-01", "dev-agent-02"));
    }

    #[test]
    fn test_glob_match_double_star_matches_anything() {
        assert!(glob_match("**", "anything"));
        assert!(glob_match("**", "/home/admin/secret"));
        assert!(glob_match("**", "simple"));
    }

    #[test]
    fn test_glob_match_single_star_matches_anything_except_slash() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", "simple"));
        assert!(glob_match("file_*.txt", "file_read.txt"));
        assert!(!glob_match("file_*.txt", "file_read/write.txt"));
    }

    #[test]
    fn test_glob_match_question_matches_single_char() {
        assert!(glob_match("file_?.txt", "file_a.txt"));
        assert!(glob_match("file_?.txt", "file_1.txt"));
        assert!(glob_match("file_?.txt", "file_z.txt"));
        assert!(!glob_match("file_?.txt", "file_ab.txt"));
        assert!(!glob_match("file_?.txt", "file_.txt"));
    }

    #[test]
    fn test_glob_match_empty_pattern() {
        assert!(!glob_match("", "anything"));
        assert!(glob_match("", ""));
    }

    #[test]
    fn test_glob_match_path_with_directory_separators() {
        assert!(glob_match("/home/admin/**", "/home/admin/code/closeclaw/src/main.rs"));
        assert!(glob_match("/home/admin/**", "/home/admin/code"));
    }

    #[test]
    fn test_glob_match_directory_star_does_not_match_slash() {
        assert!(!glob_match("*/file.txt", "dir/file.txt"));
    }

    #[test]
    fn test_glob_match_case_sensitive() {
        assert!(glob_match("File.txt", "File.txt"));
        assert!(!glob_match("File.txt", "file.txt"));
    }

    #[test]
    fn test_glob_match_nested_double_star() {
        assert!(glob_match("**/*.rs", "main.rs"));
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "src/deep/path/main.rs"));
    }

    // -------------------------------------------------------------------------
    // Rule evaluation tests
    // -------------------------------------------------------------------------

    use super::super::actions::ActionBuilder;
    use crate::permission::rules::{RuleBuilder, RuleSetBuilder};

    fn make_default_deny_ruleset() -> RuleSet {
        RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Deny)
            .default_command(Effect::Deny)
            .default_network(Effect::Deny)
            .default_inter_agent(Effect::Deny)
            .default_config(Effect::Deny)
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_permission_deny_precedence_over_allow() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("deny-cargo-reset")
                    .subject_agent("test-agent")
                    .deny()
                    .priority(1)
                    .action(
                        ActionBuilder::command("cargo")
                            .blocked_args(vec!["reset".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["reset".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_allow_non_blocked_args() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[test]
    fn test_args_match_command_args_any() {
        let engine = PermissionEngine::new(make_default_deny_ruleset());
        assert!(engine.args_match(&CommandArgs::Any, &["any".to_string(), "args".to_string()]));
        assert!(engine.args_match(&CommandArgs::Any, &[]));
    }

    #[test]
    fn test_args_match_command_args_allowed() {
        let engine = PermissionEngine::new(make_default_deny_ruleset());
        let allowed = CommandArgs::Allowed {
            allowed: vec!["build".to_string(), "test".to_string()],
        };
        assert!(engine.args_match(&allowed, &["build".to_string()]));
        assert!(engine.args_match(&allowed, &["build".to_string(), "test".to_string()]));
        assert!(!engine.args_match(&allowed, &["build".to_string(), "run".to_string()]));
    }

    #[test]
    fn test_args_match_command_args_blocked() {
        let engine = PermissionEngine::new(make_default_deny_ruleset());
        let blocked = CommandArgs::Blocked {
            blocked: vec!["reset".to_string(), "--force".to_string()],
        };
        assert!(engine.args_match(&blocked, &["reset".to_string()]));
        assert!(engine.args_match(&blocked, &["reset".to_string(), "--hard".to_string()]));
        assert!(engine.args_match(&blocked, &["commit".to_string(), "--force".to_string()]));
        assert!(!engine.args_match(&blocked, &["commit".to_string(), "push".to_string()]));
    }

    #[tokio::test]
    async fn test_permission_file_op_read_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("read-home")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/home/admin/file.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_file_op_write_denied_by_operation_mismatch() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("read-only")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/home/admin/file.txt".to_string(),
            op: "write".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_exec_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_exec_denied_command_mismatch() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_args_allowed_list() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo-build-test")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::command("cargo")
                            .allowed_args(vec!["build".to_string(), "test".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["run".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_args_blocked() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo-no-args")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_args_any() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo-any-args")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["any".to_string(), "args".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_net_op_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-internal-https")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::network()
                            .with_hosts(vec!["*.internal.corp".to_string()])
                            .with_ports(vec![443])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_network(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::NetOp {
            agent: "test-agent".to_string(),
            host: "api.internal.corp".to_string(),
            port: 443,
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::Bare(PermissionRequestBody::NetOp {
            agent: "test-agent".to_string(),
            host: "api.internal.corp".to_string(),
            port: 8080,
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_net_op_empty_hosts_matches_all() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-all-ports")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::network().with_ports(vec![443]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_network(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::NetOp {
            agent: "test-agent".to_string(),
            host: "any.host.com".to_string(),
            port: 443,
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_tool_call_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-file-ops")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::tool_call("file_ops")
                            .with_methods(vec!["read".to_string(), "write".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "test-agent".to_string(),
            skill: "file_ops".to_string(),
            method: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "test-agent".to_string(),
            skill: "file_ops".to_string(),
            method: "delete".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_tool_call_empty_methods_matches_all() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-file-ops-any-method")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::tool_call("file_ops").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "test-agent".to_string(),
            skill: "file_ops".to_string(),
            method: "any_method".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_inter_agent_msg_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-to-parent")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::inter_agent()
                            .with_agents(vec!["parent-agent".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_inter_agent(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::InterAgentMsg {
            from: "test-agent".to_string(),
            to: "parent-agent".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::Bare(PermissionRequestBody::InterAgentMsg {
            from: "test-agent".to_string(),
            to: "stranger-agent".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_inter_agent_empty_agents_matches_all() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-all-inter-agent")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::inter_agent().build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_inter_agent(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::InterAgentMsg {
            from: "test-agent".to_string(),
            to: "any-agent".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_config_write_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-config-write")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::config_write()
                            .with_files(vec!["configs/*.json".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_config(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "configs/agents.json".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "secrets/passwords.json".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_config_write_empty_files_matches_all() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-all-config-write")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::config_write().build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_config(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "any/config.json".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_subject_exact_match() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("exact-match")
                    .subject_agent("specific-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "specific-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "other-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_subject_glob_match() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("glob-match")
                    .subject_glob("test-*")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent-01".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_unknown_agent_uses_defaults() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Allow)
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);

        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "totally-unknown-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "totally-unknown-agent".to_string(),
            cmd: "ls".to_string(),
            args: vec![],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_empty_ruleset() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "any-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_rule_action_type_mismatch() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("command-only")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_unicode_in_path() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-unicode")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/home/\u{7528}\u{6237}/\u{6587}\u{4EF6}.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_denied_reason_includes_rule_name() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("my-specific-deny-rule")
                    .subject_agent("test-agent")
                    .deny()
                    .action(ActionBuilder::file("read", vec!["/secret/**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Allow)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/secret/file.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        if let PermissionResponse::Denied { reason, rule } = response {
            assert!(rule == "my-specific-deny-rule");
            assert!(reason.contains("my-specific-deny-rule"));
        } else {
            panic!("Expected Denied response");
        }
    }

    #[tokio::test]
    async fn test_permission_allowed_token_format() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-all")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Allow)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        });
        let response = engine.evaluate(request);
        if let PermissionResponse::Allowed { token } = response {
            assert!(token.starts_with("perm_"));
        } else {
            panic!("Expected Allowed response");
        }
    }

    #[tokio::test]
    async fn test_permission_multiple_deny_rules_first_wins() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("deny-all-cargo")
                    .subject_agent("multi-deny-agent")
                    .deny()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Allow)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "multi-deny-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        });
        let response = engine.evaluate(request);
        assert!(matches!(response, PermissionResponse::Denied { rule, .. } if rule == "deny-all-cargo"));
    }

    #[test]
    fn test_subject_matches_unicode() {
        let subject = Subject::AgentOnly {
            agent: "\u{65E5}\u{672C}\u{8A9E}-agent".to_string(),
            match_type: MatchType::Exact,
        };
        assert!(subject.matches(&Caller {
            user_id: "".to_string(),
            agent: "\u{65E5}\u{672C}\u{8A9E}-agent".to_string(),
            creator_id: String::new(),
        }));
        assert!(!subject.matches(&Caller {
            user_id: "".to_string(),
            agent: "other-agent".to_string(),
            creator_id: String::new(),
        }));
    }

    #[test]
    fn test_subject_matches_glob_unicode() {
        let subject = Subject::AgentOnly {
            agent: "*-agent".to_string(),
            match_type: MatchType::Glob,
        };
        assert!(subject.matches(&Caller {
            user_id: "".to_string(),
            agent: "\u{65E5}\u{672C}\u{8A9E}-agent".to_string(),
            creator_id: String::new(),
        }));
        assert!(subject.matches(&Caller {
            user_id: "".to_string(),
            agent: "test-agent".to_string(),
            creator_id: String::new(),
        }));
        assert!(!subject.matches(&Caller {
            user_id: "".to_string(),
            agent: "agent".to_string(),
            creator_id: String::new(),
        }));
    }
}
