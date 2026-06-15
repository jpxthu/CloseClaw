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
