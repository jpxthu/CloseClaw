//! Permission Engine - Core data structures
//!
//! Types, data structures, and Subject/Rule validation logic.

use super::engine_matching::glob_match;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

    /// Check if command arguments match.
    /// For Allowed: returns true if ALL request args are in the allowed list.
    /// For Blocked: returns true if ANY request arg is in the blocked list.
    pub fn args_match(&self, rule_args: &CommandArgs, request_args: &[String]) -> bool {
        match rule_args {
            CommandArgs::Any => true,
            CommandArgs::Allowed { allowed } => request_args
                .iter()
                .all(|arg| allowed.iter().any(|a| glob_match(a, arg))),
            CommandArgs::Blocked { blocked } => request_args
                .iter()
                .any(|arg| blocked.iter().any(|b| glob_match(b, arg))),
        }
    }

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
            Subject::AgentOnly { agent, match_type } => match match_type {
                MatchType::Exact => agent == &caller.agent,
                MatchType::Glob => glob_match(agent, &caller.agent),
            },
            Subject::UserAndAgent {
                user_id,
                agent,
                user_match,
                agent_match,
            } => {
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
            let json: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;

            let match_mode = json
                .get("match_mode")
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
                    let fields: UAFields =
                        serde_json::from_value(json).map_err(serde::de::Error::custom)?;
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
                    let fields: AOFields =
                        serde_json::from_value(json).map_err(serde::de::Error::custom)?;
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
    Allowed {
        allowed: Vec<String>,
    },
    Blocked {
        blocked: Vec<String>,
    },
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
    FileOp {
        agent: String,
        path: String,
        op: String,
    },
    CommandExec {
        agent: String,
        cmd: String,
        args: Vec<String>,
    },
    NetOp {
        agent: String,
        host: String,
        port: u16,
    },
    ToolCall {
        agent: String,
        skill: String,
        method: String,
    },
    InterAgentMsg {
        from: String,
        to: String,
    },
    ConfigWrite {
        agent: String,
        config_file: String,
    },
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
            PermissionRequest::Bare(body) => Caller {
                user_id: String::new(),
                agent: body.agent_id().to_string(),
                creator_id: String::new(),
            },
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
            PermissionRequest::Bare(body) => PermissionRequest::WithCaller {
                caller,
                request: body,
            },
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
