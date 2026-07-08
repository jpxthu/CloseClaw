//! Whitelist Rule Builder
//!
//! Converts approval request details into persistent whitelist rules.
//!
//! When an owner approves an operation with [`ApprovalMode::WithWhitelist`],
//! this module constructs a `Rule` that the permission engine will evaluate
//! as `Allow` on subsequent matching operations.
//!
//! # Mapping
//!
//! - [`PermissionRequestBody`] → [`Action`] (mismatched types return `None`)
//! - [`Caller`] → [`Subject`] (Owner callers produce `AgentOnly` subjects)
//! - Both combined → [`Rule`] with `Effect::Allow`

use std::path::Path;

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
    }
}

/// Convert a [`Caller`] into the appropriate [`Subject`].
///
/// - Owner (`user_id == "owner"`) → [`Subject::AgentOnly`] (Owner only
///   governs the agent dimension)
/// - Non-empty `user_id` → [`Subject::UserAndAgent`] (dual-key matching)
/// - Empty `user_id` → [`Subject::AgentOnly`]
pub fn caller_to_subject(caller: &Caller) -> Subject {
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

/// Build a whitelist [`Rule`] from caller and request body.
///
/// Returns `None` when the request body has no meaningful action mapping
/// (e.g. `ConfigWrite`, `SlashCommand`).
///
/// The generated rule has `Effect::Allow` and a caller-derived subject.
pub fn build_whitelist_rule(
    caller: &Caller,
    body: &PermissionRequestBody,
    name: &str,
) -> Option<Rule> {
    let action = request_body_to_action(body)?;
    let subject = caller_to_subject(caller);

    Some(Rule {
        name: name.to_string(),
        subject,
        effect: Effect::Allow,
        actions: vec![action],
        template: None,
        priority: 0,
    })
}

/// Append a whitelist [`Rule`] to the agent's `permissions.json`.
///
/// Path: `{config_dir}/agents/{agent_id}/permissions.json`
///
/// Reads the existing file (or starts with an empty [`RuleSet`]),
/// appends the rule, and writes it back as pretty-printed JSON.
pub fn append_whitelist_rule(config_dir: &Path, agent_id: &str, rule: Rule) -> std::io::Result<()> {
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

/// Load a [`RuleSet`] from disk, returning an empty one on missing/corrupt file.
fn load_ruleset(path: &Path) -> RuleSet {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return RuleSet::default(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}
