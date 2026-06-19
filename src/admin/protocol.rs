//! Admin RPC protocol types.
//!
//! Defines the request and response enums for CLI-to-daemon
//! communication over a Unix domain socket.
//!
//! Uses length-prefixed JSON frames:
//! ```text
//! [4-byte big-endian length (u32)][JSON frame bytes]
//! ```

use serde::{Deserialize, Serialize};

/// Information about a registered agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInfo {
    /// Agent identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Model identifier, if configured.
    pub model: Option<String>,
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
    AgentInfo { name: String },
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
    /// Detailed agent info.
    AgentInfoResult {
        id: String,
        name: String,
        model: Option<String>,
        skills: Vec<String>,
    },
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
            name: "test-agent".to_string(),
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
        let resp = AdminResponse::AgentInfoResult {
            id: "agent1".to_string(),
            name: "Agent One".to_string(),
            model: Some("gpt-4".to_string()),
            skills: vec!["skill-a".to_string(), "skill-b".to_string()],
        };
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
