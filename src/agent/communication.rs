//! Agent communication permission checks.

use crate::agent::config::AgentConfig;
use serde::{Deserialize, Serialize};

/// Communication whitelist for an agent.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CommunicationConfig {
    /// Agent IDs this agent is allowed to send messages to.
    #[serde(default)]
    pub outbound: Vec<String>,
    /// Agent IDs this agent is allowed to receive messages from.
    #[serde(default)]
    pub inbound: Vec<String>,
}

impl CommunicationConfig {
    /// Returns the default communication config: only parent is allowed.
    pub fn default_with_parent(parent_id: Option<&str>) -> Self {
        let parent_list: Vec<String> = parent_id.map(String::from).into_iter().collect();
        Self {
            outbound: parent_list.clone(),
            inbound: parent_list,
        }
    }

    /// Check if communication to `target_id` is allowed.
    pub fn can_send_to(&self, target_id: &str) -> bool {
        self.outbound.iter().any(|id| id == target_id || id == "*")
    }

    /// Check if communication from `source_id` is allowed.
    pub fn can_receive_from(&self, source_id: &str) -> bool {
        self.inbound.iter().any(|id| id == source_id || id == "*")
    }
}

/// Result of a communication permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommunicationCheckResult {
    /// Communication is allowed.
    Allowed,
    /// Source agent not in target's inbound list.
    SourceNotInTargetInbound,
    /// Target agent not in source's outbound list.
    TargetNotInSourceOutbound,
}

/// Check if communication from source to target is allowed.
/// Returns the result of the central arbiter logic.
pub fn check_communication_allowed(
    source_config: &AgentConfig,
    target_config: &AgentConfig,
) -> CommunicationCheckResult {
    if !source_config.communication.can_send_to(&target_config.id) {
        return CommunicationCheckResult::TargetNotInSourceOutbound;
    }

    if !target_config
        .communication
        .can_receive_from(&source_config.id)
    {
        return CommunicationCheckResult::SourceNotInTargetInbound;
    }

    CommunicationCheckResult::Allowed
}

/// Result of a max_depth permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaxDepthCheckResult {
    /// Agent can spawn children at this depth.
    Allowed {
        current_depth: u32,
        max_allowed: u32,
    },
    /// Agent cannot spawn: would exceed max_child_depth.
    ExceedsMaxDepth {
        current_depth: u32,
        max_child_depth: u32,
    },
}

/// Check if an agent can spawn a child at the proposed depth.
///
/// `get_parent` is a callback that returns the parent agent's config given its ID.
/// Returns the result of the depth check.
pub fn check_max_depth<F>(agent_config: &AgentConfig, get_parent: F) -> MaxDepthCheckResult
where
    F: Fn(&str) -> Option<AgentConfig>,
{
    let mut current_depth = 0u32;
    let mut current_parent_id = agent_config.parent_id.clone();

    while let Some(parent_id) = current_parent_id {
        if parent_id.is_empty() {
            break;
        }
        if let Some(parent) = get_parent(&parent_id) {
            current_depth += 1;
            current_parent_id = parent.parent_id.clone();
        } else {
            break;
        }
    }

    let max_child_depth = agent_config.max_child_depth;

    if current_depth >= max_child_depth {
        MaxDepthCheckResult::ExceedsMaxDepth {
            current_depth,
            max_child_depth,
        }
    } else {
        let max_allowed = max_child_depth.saturating_sub(1);
        MaxDepthCheckResult::Allowed {
            current_depth,
            max_allowed,
        }
    }
}
