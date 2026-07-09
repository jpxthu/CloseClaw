//! Permission Engine - Helper utilities.

use super::engine_types::RuleSet;
use super::engine_types::{Action, Effect, Subject};
use closeclaw_common::SessionLookup;
use closeclaw_config::agents::AgentPermissionProvider;
use closeclaw_config::agents::AgentPermissions;
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

/// Collect AgentOnly Deny subjects from all ancestor agents in the parent
/// session chain. Traverses upward via `SessionManager::get_parent_of`,
/// collecting each ancestor's AgentOnly Deny rules from the RuleSet and
/// replacing their agent field with `child_agent_id`. Deduplicates results
/// by `(agent, match_type)`.
///
/// This implements the design doc requirement: "沿 spawn 链每增加一级深度，
/// Deny 约束集只增不减".
pub async fn collect_chain_deny_subjects(
    session_manager: &dyn SessionLookup,
    rules: &RuleSet,
    parent_session_id: &str,
    child_agent_id: &str,
) -> Vec<Subject> {
    let mut all_subjects: Vec<Subject> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut current_session = parent_session_id.to_string();

    loop {
        let parent_agent_id = match session_manager.get_chat_id(&current_session).await {
            Some(id) => id,
            None => break,
        };

        let subjects = get_agent_deny_subjects(rules, &parent_agent_id, child_agent_id);
        for subject in subjects {
            match &subject {
                Subject::AgentOnly { agent, match_type } => {
                    let key = (agent.clone(), format!("{:?}", match_type));
                    if seen.insert(key) {
                        all_subjects.push(subject);
                    }
                }
                _ => {
                    all_subjects.push(subject);
                }
            }
        }

        match session_manager.get_parent_of(&current_session).await {
            Some(parent_id) => current_session = parent_id,
            None => break,
        }
    }

    all_subjects
}

/// Compute the effective permissions for a child agent by intersecting
/// configured permissions of every ancestor in the spawn chain.
///
/// Traverses upward from the direct parent via `SessionManager::get_parent_of`.
/// At each level the ancestor's configured permissions (from
/// `agent_permissions`) are intersected with the accumulated result.
/// Ancestors without configured permissions are skipped.
///
/// Returns `None` if the direct parent has no configured permissions
/// (caller should treat as no restriction, matching prior behavior).
pub async fn collect_chain_effective_permissions(
    session_manager: &dyn SessionLookup,
    agent_permissions: &dyn AgentPermissionProvider,
    parent_session_id: &str,
    parent_agent_id: &str,
) -> Option<AgentPermissions> {
    let mut result = agent_permissions.get(parent_agent_id)?.clone();
    let mut current_session = parent_session_id.to_string();

    loop {
        let ancestor_session = match session_manager.get_parent_of(&current_session).await {
            Some(id) => id,
            None => break,
        };

        let ancestor_agent_id = match session_manager.get_chat_id(&ancestor_session).await {
            Some(id) => id,
            None => break,
        };

        if let Some(ancestor_perms) = agent_permissions.get(&ancestor_agent_id) {
            result = result.intersect(&ancestor_perms);
        }

        current_session = ancestor_session;
    }

    Some(result)
}

/// Resolve template actions with overrides applied.
pub fn resolve_template_actions(
    tmpl: &crate::templates::Template,
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
