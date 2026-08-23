//! Admin RPC protocol types.
//!
//! Defines the request and response enums for CLI-to-daemon
//! communication over a Unix domain socket.
//!
//! Uses length-prefixed JSON frames:
//! ```text
//! [4-byte big-endian length (u32)][JSON frame bytes]
//! ```

use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::{MemoryConfig, ModelSpec, SubagentsConfig};
use serde::{Deserialize, Serialize};

/// Information about a registered agent (summary for list).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInfo {
    /// Agent identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Model identifier, if configured.
    pub model: Option<String>,
}

/// Full agent configuration profile.
///
/// Serde field names use camelCase to match `agent-config.md` field table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfoResult {
    /// Agent unique identifier.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Parent agent ID, if any.
    pub parent_id: Option<String>,
    /// Default LLM model.
    pub model: Option<ModelSpec>,
    /// Working directory path.
    pub workspace: Option<String>,
    /// Bootstrap files directory path.
    pub agent_dir: Option<String>,
    /// Bootstrap file loading mode.
    pub bootstrap_mode: BootstrapMode,
    /// Available skill names.
    pub skills: Vec<String>,
    /// Available tool names.
    pub tools: Vec<String>,
    /// Disallowed tool names.
    pub disallowed_tools: Vec<String>,
    /// Sub-agent spawn control parameters.
    pub subagents: SubagentsConfig,
    /// Memory subsystem configuration.
    pub memory: Option<MemoryConfig>,
}

/// Information about a registered skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillInfo {
    /// Skill name.
    pub name: String,
    /// Skill version string, if available.
    pub version: Option<String>,
}

/// Request sent from the CLI client to the admin server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminRequest {
    /// List all registered agents.
    AgentList,
    /// Get detailed info for a specific agent.
    AgentInfo { id: String },
    /// Create a new agent with the given name and optional model.
    AgentCreate { name: String, model: Option<String> },
    /// List all installed skills.
    SkillList,
    /// Install a skill by name.
    SkillInstall { name: String },
    /// Health check — returns Pong.
    Ping,
}

/// Response sent from the admin server back to the CLI client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminResponse {
    /// List of agents.
    AgentListResult { agents: Vec<AgentInfo> },
    /// Detailed agent info (full configuration profile).
    AgentInfoResult(Box<AgentInfoResult>),
    /// List of skills.
    SkillListResult { skills: Vec<SkillInfo> },
    /// Operation succeeded.
    Ok,
    /// Operation failed.
    Error { message: String },
    /// Health check acknowledgement.
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_list_request_serialization() {
        let req = AdminRequest::AgentList;
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: AdminRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_agent_info_request_serialization() {
        let req = AdminRequest::AgentInfo {
            id: "test-agent".to_string(),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: AdminRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_agent_create_request_serialization() {
        let req = AdminRequest::AgentCreate {
            name: "new-agent".to_string(),
            model: Some("gpt-4".to_string()),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: AdminRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_agent_create_request_no_model() {
        let req = AdminRequest::AgentCreate {
            name: "new-agent".to_string(),
            model: None,
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: AdminRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_skill_list_request_serialization() {
        let req = AdminRequest::SkillList;
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: AdminRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_skill_install_request_serialization() {
        let req = AdminRequest::SkillInstall {
            name: "my-skill".to_string(),
        };
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: AdminRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_ping_request_serialization() {
        let req = AdminRequest::Ping;
        let json = serde_json::to_vec(&req).unwrap();
        let deserialized: AdminRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_agent_list_response_serialization() {
        let resp = AdminResponse::AgentListResult {
            agents: vec![
                AgentInfo {
                    id: "agent1".to_string(),
                    name: "Agent One".to_string(),
                    model: Some("gpt-4".to_string()),
                },
                AgentInfo {
                    id: "agent2".to_string(),
                    name: "Agent Two".to_string(),
                    model: None,
                },
            ],
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: AdminResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_agent_info_response_serialization() {
        use closeclaw_config::agents::SubagentsConfig;
        let resp = AdminResponse::AgentInfoResult(Box::new(AgentInfoResult {
            id: "agent1".to_string(),
            name: "Agent One".to_string(),
            parent_id: None,
            model: Some(closeclaw_config::agents::ModelSpec::single("gpt-4")),
            workspace: None,
            agent_dir: None,
            bootstrap_mode: closeclaw_common::BootstrapMode::Full,
            skills: vec!["skill-a".to_string(), "skill-b".to_string()],
            tools: vec!["*".to_string()],
            disallowed_tools: vec![],
            subagents: SubagentsConfig::default(),
            memory: None,
        }));
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: AdminResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_skill_list_response_serialization() {
        let resp = AdminResponse::SkillListResult {
            skills: vec![
                SkillInfo {
                    name: "skill-a".to_string(),
                    version: Some("1.0.0".to_string()),
                },
                SkillInfo {
                    name: "skill-b".to_string(),
                    version: None,
                },
            ],
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: AdminResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_ok_response_serialization() {
        let resp = AdminResponse::Ok;
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: AdminResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_error_response_serialization() {
        let resp = AdminResponse::Error {
            message: "something went wrong".to_string(),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: AdminResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }

    #[test]
    fn test_pong_response_serialization() {
        let resp = AdminResponse::Pong;
        let json = serde_json::to_vec(&resp).unwrap();
        let deserialized: AdminResponse = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            serde_json::to_string(&deserialized).unwrap()
        );
    }
}
