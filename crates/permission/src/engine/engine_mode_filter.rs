//! Plan Mode and Auto Mode runtime filters for PermissionEngine.
//!
//! Extracted from `engine_eval.rs` to keep that file under the
//! 1000-line limit.

use super::engine_eval::PermissionEngine;
use super::engine_helpers::generate_token;
use super::engine_risk::{assess_risk_level, RiskLevel};
use super::engine_types::{
    MessageDirection, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_common::session_mode::SessionMode;
use tracing::info;

impl PermissionEngine {
    /// Plan Mode write-operation filtering.
    ///
    /// When the agent's session mode is `Plan`, the following operations
    /// are denied:
    /// - `FileOp` with op = "write" (unless the path is under plans/)
    /// - `CommandExec`
    /// - `ConfigWrite`
    ///
    /// Returns `Some(Denied)` if the operation should be blocked,
    /// `None` to proceed with normal evaluation.
    pub(super) fn check_plan_mode_filter(
        &self,
        request: &PermissionRequest,
        agent_id: &str,
    ) -> Option<PermissionResponse> {
        let query_ref = self.session_mode_query();
        let query = query_ref.as_ref()?;
        let mode = query.get_session_mode(agent_id)?;
        if mode != SessionMode::Plan {
            return None;
        }

        let body = request.body();
        match body {
            PermissionRequestBody::FileOp { op, path, .. } if op == "write" => {
                if is_plans_path(path) {
                    return None;
                }
                info!(
                    agent = agent_id,
                    result = "denied",
                    reason = "plan_mode_write_denied",
                    path = %path,
                    "permission check completed"
                );
                Some(PermissionResponse::Denied {
                    reason: "write operation denied in Plan mode".to_string(),
                    rule: "<plan_mode_filter>".to_string(),
                    risk_level: assess_risk_level(body),
                })
            }
            PermissionRequestBody::CommandExec { .. } => {
                info!(
                    agent = agent_id,
                    result = "denied",
                    reason = "plan_mode_command_denied",
                    "permission check completed"
                );
                Some(PermissionResponse::Denied {
                    reason: "command execution denied in Plan mode".to_string(),
                    rule: "<plan_mode_filter>".to_string(),
                    risk_level: assess_risk_level(body),
                })
            }
            PermissionRequestBody::ConfigWrite { .. } => {
                info!(
                    agent = agent_id,
                    result = "denied",
                    reason = "plan_mode_config_write_denied",
                    "permission check completed"
                );
                Some(PermissionResponse::Denied {
                    reason: "config write denied in Plan mode".to_string(),
                    rule: "<plan_mode_filter>".to_string(),
                    risk_level: assess_risk_level(body),
                })
            }
            PermissionRequestBody::ToolCall { skill, .. } if skill == "ask_user_question" => {
                info!(
                    agent = agent_id,
                    result = "allowed_with_context",
                    reason = "plan_mode_ask_user_question_clarification_only",
                    "permission check completed"
                );
                Some(PermissionResponse::Allowed {
                    token: generate_token(),
                    context_modifier: Some(
                        "[plan_mode_context] AskUserQuestion \
                         is for requirement clarification only. \
                         Do NOT use it as an approval substitute."
                            .to_string(),
                    ),
                })
            }
            _ => None,
        }
    }

    /// Auto Mode runtime dangerous-operation review.
    ///
    /// Design doc: "Auto Mode 下完整工具集可见，但危险操作需运行时审查"
    /// and "不擅自向外部平台发送消息". Dangerous operations (High/Critical
    /// risk) and outgoing MessageSend are denied in Auto Mode.
    ///
    /// Owner is exempt.
    ///
    /// Returns `Some(Denied)` if the operation should be blocked,
    /// `None` to proceed with normal evaluation.
    pub(super) fn check_auto_mode_filter(
        &self,
        request: &PermissionRequest,
        agent_id: &str,
    ) -> Option<PermissionResponse> {
        let query_ref = self.session_mode_query();
        let query = query_ref.as_ref()?;
        let mode = query.get_session_mode(agent_id)?;
        if mode != SessionMode::Auto {
            return None;
        }

        let body = request.body();

        let risk = assess_risk_level(body);
        if risk.is_high_or_critical() {
            info!(
                agent = agent_id,
                result = "denied",
                reason = "auto_mode_risk_gate",
                risk_level = ?risk,
                "permission check completed"
            );
            return Some(PermissionResponse::Denied {
                reason: "Auto Mode: dangerous operation requires \
                         user approval"
                    .to_string(),
                rule: "<auto_mode_filter>".to_string(),
                risk_level: risk,
            });
        }

        if let PermissionRequestBody::MessageSend {
            direction: MessageDirection::Send,
            ..
        } = body
        {
            info!(
                agent = agent_id,
                result = "denied",
                reason = "auto_mode_message_send_denied",
                "permission check completed"
            );
            return Some(PermissionResponse::Denied {
                reason: "Auto Mode: proactive message sending \
                         is not allowed"
                    .to_string(),
                rule: "<auto_mode_filter>".to_string(),
                risk_level: RiskLevel::Low,
            });
        }

        None
    }
}

/// Check if a file path belongs to the plans/ directory.
fn is_plans_path(path: &str) -> bool {
    path.starts_with("plans/") || path.contains("/plans/")
}
