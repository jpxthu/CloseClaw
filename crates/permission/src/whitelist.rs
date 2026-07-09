//! Whitelist Rule Builder
//!
//! Converts approval request details into persistent whitelist rules.
//!
//! When an owner approves an operation with [`ApprovalMode::WithWhitelist { .. }`],
//! this module constructs a `Rule` that the permission engine will evaluate
//! as `Allow` on subsequent matching operations.
//!
//! # Mapping
//!
//! - [`PermissionRequestBody`] → [`Action`] (mismatched types return `None`)
//! - [`Caller`] → [`Subject`] (Owner callers produce `AgentOnly` subjects)
//! - Both combined → [`Rule`] with `Effect::Allow`

use std::path::Path;

use crate::approval::WhitelistTarget;
use crate::engine::engine_types::{
    Action, Caller, CommandArgs, Effect, PermissionRequestBody, Rule, RuleSet, Subject,
};

/// Convert a [`PermissionRequestBody`] into the corresponding [`Action`].
///
/// Returns `None` for request types that have no meaningful action mapping:
/// - [`PermissionRequestBody::ConfigWrite`]: always high-risk, never whitelisted
/// - [`PermissionRequestBody::SlashCommand`]: no corresponding action dimension
pub fn request_body_to_action(body: &PermissionRequestBody) -> Option<Action> {
    match body {
        PermissionRequestBody::FileOp { path, op, .. } => Some(Action::File {
            operation: op.clone(),
            paths: vec![path.clone()],
        }),
        PermissionRequestBody::CommandExec { cmd, args, .. } => Some(Action::Command {
            command: cmd.clone(),
            args: CommandArgs::Allowed {
                allowed: args.clone(),
            },
        }),
        PermissionRequestBody::NetOp { host, port, .. } => Some(Action::Network {
            hosts: vec![host.clone()],
            ports: vec![*port],
        }),
        PermissionRequestBody::ToolCall { skill, method, .. } => Some(Action::ToolCall {
            skill: skill.clone(),
            methods: vec![method.clone()],
        }),
        PermissionRequestBody::InterAgentMsg { to, .. } => Some(Action::InterAgent {
            agents: vec![to.clone()],
        }),
        // ConfigWrite: high-risk, never reaches whitelist
        PermissionRequestBody::ConfigWrite { .. } => None,
        // SlashCommand: no corresponding action dimension
        PermissionRequestBody::SlashCommand { .. } => None,
        // MessageSend: message permission dimension
        PermissionRequestBody::MessageSend {
            direction, target, ..
        } => Some(Action::Message {
            direction: direction.clone(),
            targets: vec![target.clone()],
        }),
    }
}

/// Convert a [`Caller`] into the appropriate [`Subject`], guided by
/// [`WhitelistTarget`].
///
/// - [`WhitelistTarget::Auto`]: Owner → `AgentOnly`, non-owner → `UserAndAgent`
/// - [`WhitelistTarget::AgentOnly`]: always `AgentOnly`
/// - [`WhitelistTarget::UserAndAgent`]: `UserAndAgent` when `user_id` is
///   non-empty, otherwise fallback to `AgentOnly`
pub fn caller_to_subject(caller: &Caller, target: WhitelistTarget) -> Subject {
    match target {
        WhitelistTarget::Auto => {
            if caller.user_id == "owner" || caller.user_id.is_empty() {
                Subject::AgentOnly {
                    agent: caller.agent.clone(),
                    match_type: Default::default(),
                }
            } else {
                Subject::UserAndAgent {
                    user_id: caller.user_id.clone(),
                    agent: caller.agent.clone(),
                    user_match: Default::default(),
                    agent_match: Default::default(),
                }
            }
        }
        WhitelistTarget::AgentOnly => Subject::AgentOnly {
            agent: caller.agent.clone(),
            match_type: Default::default(),
        },
        WhitelistTarget::UserAndAgent => {
            if caller.user_id.is_empty() {
                Subject::AgentOnly {
                    agent: caller.agent.clone(),
                    match_type: Default::default(),
                }
            } else {
                Subject::UserAndAgent {
                    user_id: caller.user_id.clone(),
                    agent: caller.agent.clone(),
                    user_match: Default::default(),
                    agent_match: Default::default(),
                }
            }
        }
    }
}

/// Build a whitelist [`Rule`] from caller and request body.
///
/// Returns `None` when the request body has no meaningful action mapping
/// (e.g. `ConfigWrite`, `SlashCommand`).
///
/// The generated rule has `Effect::Allow` and a caller-derived subject,
/// with [`WhitelistTarget`] controlling the subject type.
pub fn build_whitelist_rule(
    caller: &Caller,
    body: &PermissionRequestBody,
    name: &str,
    target: WhitelistTarget,
) -> Option<Rule> {
    let action = request_body_to_action(body)?;
    let subject = caller_to_subject(caller, target);

    Some(Rule {
        name: name.to_string(),
        subject,
        effect: Effect::Allow,
        actions: vec![action],
        template: None,
        priority: 0,
    })
}

/// Build a deny [`Rule`] from caller and request body.
///
/// Symmetric to [`build_whitelist_rule`], but generates `Effect::Deny`.
/// Returns `None` for request types with no meaningful action mapping.
pub fn build_deny_rule(
    caller: &Caller,
    body: &PermissionRequestBody,
    name: &str,
    target: WhitelistTarget,
) -> Option<Rule> {
    let action = request_body_to_action(body)?;
    let subject = caller_to_subject(caller, target);

    Some(Rule {
        name: name.to_string(),
        subject,
        effect: Effect::Deny,
        actions: vec![action],
        template: None,
        priority: 0,
    })
}

/// Append a [`Rule`] to the agent's `permissions.json`.
///
/// Path: `{config_dir}/agents/{agent_id}/permissions.json`
///
/// Reads the existing file (or starts with an empty [`RuleSet`]),
/// appends the rule, and writes it back as pretty-printed JSON.
pub fn append_rule(config_dir: &Path, agent_id: &str, rule: Rule) -> std::io::Result<()> {
    let path = config_dir
        .join("agents")
        .join(agent_id)
        .join("permissions.json");

    let mut ruleset = load_ruleset(&path);
    ruleset.rules.push(rule);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&ruleset).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)
}

/// Append a whitelist (allow) [`Rule`] to the agent's `permissions.json`.
///
/// Convenience wrapper over [`append_rule`].
pub fn append_whitelist_rule(config_dir: &Path, agent_id: &str, rule: Rule) -> std::io::Result<()> {
    append_rule(config_dir, agent_id, rule)
}

/// Append a deny [`Rule`] to the agent's `permissions.json`.
///
/// Convenience wrapper over [`append_rule`].
pub fn append_deny_rule(config_dir: &Path, agent_id: &str, rule: Rule) -> std::io::Result<()> {
    append_rule(config_dir, agent_id, rule)
}

/// Load a [`RuleSet`] from disk, returning an empty one on missing/corrupt file.
fn load_ruleset(path: &Path) -> RuleSet {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return RuleSet::default(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}
