//! Permission Engine - Helper utilities.

use super::engine_types::{Action, Effect, Subject};
use crate::permission::engine::engine_types::RuleSet;
use std::collections::HashMap;

/// Extract AgentOnly + Deny subjects from parent agent, replacing agent with child_agent_id.
/// Used for sub-agent permission inheritance via parent-agent deny propagation.
pub fn get_agent_deny_subjects(
    rules: &RuleSet,
    parent_agent_id: &str,
    child_agent_id: &str,
) -> Vec<Subject> {
    rules
        .rules
        .iter()
        .filter(|rule| {
            if rule.effect != Effect::Deny {
                return false;
            }
            match &rule.subject {
                Subject::AgentOnly { agent, .. } => agent == parent_agent_id,
                Subject::UserAndAgent { .. } => false,
            }
        })
        .map(|rule| match &rule.subject {
            Subject::AgentOnly {
                agent: _,
                match_type,
            } => Subject::AgentOnly {
                agent: child_agent_id.to_string(),
                match_type: match_type.clone(),
            },
            Subject::UserAndAgent { .. } => unreachable!(),
        })
        .collect()
}

/// Resolve template actions with overrides applied.
pub fn resolve_template_actions(
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

/// Generate a short-lived permission token.
pub fn generate_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("perm_{}_{:016x}", duration.as_secs(), rand_u64())
}

fn rand_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish()
}
