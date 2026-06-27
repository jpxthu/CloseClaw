//! Simplified permission check — convenience method for quick agent action evaluation.

use super::engine_eval::PermissionEngine;
use super::engine_risk::assess_risk_level;
use super::engine_types::{PermissionRequest, PermissionRequestBody, PermissionResponse, Subject};

impl PermissionEngine {
    /// Simplified permission check — evaluates if `agent_id` may
    /// perform `action`.
    pub fn check(
        &self,
        agent_id: &str,
        action: &str,
        extra_deny_subjects: Option<&[Subject]>,
    ) -> PermissionResponse {
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
            _ => {
                let body = PermissionRequestBody::ToolCall {
                    agent: agent_id.to_string(),
                    skill: action.to_string(),
                    method: "unknown".to_string(),
                };
                return PermissionResponse::Denied {
                    reason: format!("unknown action: {}", action),
                    rule: "<check>".to_string(),
                    risk_level: assess_risk_level(&body),
                };
            }
        };

        self.evaluate(
            PermissionRequest::Bare(body),
            extra_deny_subjects.map(|s| s.to_vec()),
        )
    }
}
