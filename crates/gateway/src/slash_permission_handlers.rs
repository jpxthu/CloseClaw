//! Gateway-level permission command handlers.
//!
//! Contains the [`Gateway`] methods that handle `/perm`, `/user approve`,
//! `/user reject`, and new-user-registration logic.  These are intercepted
//! by the Gateway *before* reaching the [`SlashDispatcher`], consistent
//! with the design doc that permission operations belong to the Gateway
//! layer.

use std::sync::Arc;

use closeclaw_permission::approval::WhitelistTarget;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{Caller, PermissionRequestBody};
use closeclaw_permission::UserRegistry;
use tokio::sync::RwLock;

use super::{Gateway, HandleResult};

// ── Gateway permission command interception ───────────────────────────
// Permission commands are intercepted here before reaching the
// SlashDispatcher, consistent with the design doc that permission
// operations belong to the Gateway layer.

impl Gateway {
    /// Handle a permission operation — owner-only permission rule management.
    pub(crate) async fn handle_permission_op(
        &self,
        op: &closeclaw_common::PermissionOperation,
        sender_id: Option<&str>,
    ) -> String {
        if sender_id != Some("owner") {
            return "权限不足：仅 Owner 可以执行权限管理操作".to_owned();
        }

        // Path traversal validation for file operations.
        if let Some(paths) = Self::op_file_paths(op) {
            for path in paths {
                if Self::is_path_dangerous(path) {
                    return format!("拒绝：路径包含危险模式 '{path}'");
                }
            }
        }

        let (rule, agent_id) = match Self::build_rule_from_op(op) {
            Some(v) => v,
            None => return "错误：无法构建规则".to_owned(),
        };

        let config_dir = match self.get_config_dir().await {
            Some(dir) => dir,
            None => return "错误：config_dir 未配置".to_owned(),
        };
        if let Err(e) = closeclaw_permission::whitelist::append_rule(&config_dir, &agent_id, rule) {
            return format!("错误：写入规则失败 — {e}");
        }

        // Hot-reload the permission engine with the updated ruleset.
        self.hot_reload_engine(&self.permission_engine, &config_dir, &agent_id)
            .await;

        format!("✅ 已执行：{}", op.describe())
    }

    fn op_file_paths(op: &closeclaw_common::PermissionOperation) -> Option<Vec<&String>> {
        match op {
            closeclaw_common::PermissionOperation::AddFileWhitelist { paths, .. }
            | closeclaw_common::PermissionOperation::AddFileDeny { paths, .. } => {
                Some(paths.iter().collect())
            }
            _ => None,
        }
    }

    fn build_rule_from_op(
        op: &closeclaw_common::PermissionOperation,
    ) -> Option<(closeclaw_permission::Rule, String)> {
        let whitelist = matches!(
            op,
            closeclaw_common::PermissionOperation::AddFileWhitelist { .. }
                | closeclaw_common::PermissionOperation::AddCommandWhitelist { .. }
        );
        let (agent, body, name) = match op {
            closeclaw_common::PermissionOperation::AddFileWhitelist { agent, op, paths }
            | closeclaw_common::PermissionOperation::AddFileDeny { agent, op, paths } => {
                let prefix = if whitelist { "allow" } else { "deny" };
                (
                    agent.clone(),
                    PermissionRequestBody::FileOp {
                        agent: agent.clone(),
                        path: paths.join(","),
                        op: op.clone(),
                    },
                    format!("perm-{prefix}-file-{agent}"),
                )
            }
            closeclaw_common::PermissionOperation::AddCommandWhitelist {
                agent,
                command,
                args,
                ..
            }
            | closeclaw_common::PermissionOperation::AddCommandDeny {
                agent,
                command,
                args,
                ..
            } => {
                let prefix = if whitelist { "allow" } else { "deny" };
                (
                    agent.clone(),
                    PermissionRequestBody::CommandExec {
                        agent: agent.clone(),
                        cmd: command.clone(),
                        args: args.clone(),
                    },
                    format!("perm-{prefix}-cmd-{agent}"),
                )
            }
            // CreateUser goes through the ApprovalFlow, not through
            // the whitelist/deny rule path.
            closeclaw_common::PermissionOperation::CreateUser { .. } => {
                return None;
            }
        };
        let caller = Caller {
            user_id: "owner".to_owned(),
            agent: agent.clone(),
            creator_id: String::new(),
        };
        let rule_fn = if whitelist {
            closeclaw_permission::whitelist::build_whitelist_rule
        } else {
            closeclaw_permission::whitelist::build_deny_rule
        };
        rule_fn(&caller, &body, &name, WhitelistTarget::Auto).map(|r| (r, agent))
    }

    pub(crate) async fn hot_reload_engine(
        &self,
        permission_engine: &RwLock<Option<Arc<tokio::sync::RwLock<PermissionEngine>>>>,
        config_dir: &std::path::Path,
        agent_id: &str,
    ) {
        let path = config_dir
            .join("agents")
            .join(agent_id)
            .join("permissions.json");
        if let Ok(json) = tokio::fs::read_to_string(&path).await {
            if let Ok(ruleset) = serde_json::from_str::<closeclaw_permission::RuleSet>(&json) {
                if let Some(engine_arc) = permission_engine.read().await.as_ref() {
                    if let Ok(mut engine) = engine_arc.try_write() {
                        engine.reload_rules(ruleset);
                    }
                }
            }
        }
    }

    pub(crate) fn is_path_dangerous(path: &str) -> bool {
        path.contains("../")
            || path.contains("..\\")
            || path.starts_with('/')
            || (path.len() >= 2 && path.as_bytes()[1] == b':')
            || path.contains('\0')
    }

    /// Handle a `UserApprove` result — register the user via ApprovalFlow.
    pub(crate) async fn handle_user_approve(
        &self,
        request_id: &str,
        initial_permissions: &[closeclaw_common::permission_op::InitialPermissionSet],
        sender_id: Option<&str>,
    ) -> String {
        if sender_id != Some("owner") {
            return "权限不足：仅 Owner 可以审批用户注册".to_owned();
        }

        let flow_guard = self.approval_flow.read().await;
        let Some(flow_arc) = flow_guard.as_ref() else {
            return "错误：审批流未配置".to_owned();
        };
        let mut flow = flow_arc.lock().await;
        // Set the selected initial permissions on the pending request.
        flow.set_user_creation_permissions(request_id, initial_permissions.to_vec());
        match flow
            .approve_request(
                request_id,
                closeclaw_permission::approval::ApprovalMode::Once,
            )
            .await
        {
            Ok(true) => {
                let perms: Vec<&str> = initial_permissions.iter().map(|p| p.label()).collect();
                format!("✅ 用户注册已批准（权限: [{}]）", perms.join(", "))
            }
            Ok(false) => "用户注册审批失败：用户可能已注册".to_owned(),
            Err(e) => format!("用户注册审批失败：{:?}", e),
        }
    }

    /// Handle a `UserReject` result — reject the user registration via ApprovalFlow.
    pub(crate) async fn handle_user_reject(
        &self,
        request_id: &str,
        sender_id: Option<&str>,
    ) -> String {
        if sender_id != Some("owner") {
            return "权限不足：仅 Owner 可以拒绝用户注册".to_owned();
        }

        let flow_guard = self.approval_flow.read().await;
        let Some(flow_arc) = flow_guard.as_ref() else {
            return "错误：审批流未配置".to_owned();
        };
        let mut flow = flow_arc.lock().await;
        if flow.deny_request(request_id) {
            "用户注册已拒绝".to_owned()
        } else {
            "拒绝失败：请求不存在或已处理".to_owned()
        }
    }

    /// Handle `/perm` command directly in the Gateway.
    pub(crate) async fn handle_perm_cmd(&self, args: &str, sender_id: Option<&str>) -> String {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.is_empty() {
            return Self::perm_usage();
        }
        match parts[0] {
            "allow-file" => self.handle_perm_file_op(&parts, true, sender_id).await,
            "deny-file" => self.handle_perm_file_op(&parts, false, sender_id).await,
            "allow-cmd" => self.handle_perm_cmd_op(&parts, true, sender_id).await,
            "deny-cmd" => self.handle_perm_cmd_op(&parts, false, sender_id).await,
            other => format!("未知子命令：{other}\n\n{}", Self::perm_usage()),
        }
    }

    pub(crate) fn perm_usage() -> String {
        "用法：\n\
         /perm allow-file <agent> <op> <paths...>\n\
         /perm deny-file <agent> <op> <paths...>\n\
         /perm allow-cmd <agent> <command> [args...]\n\
         /perm deny-cmd <agent> <command> [args...]"
            .to_owned()
    }

    async fn handle_perm_file_op(
        &self,
        parts: &[&str],
        allow: bool,
        sender_id: Option<&str>,
    ) -> String {
        if parts.len() < 4 {
            return format!(
                "参数不足：{} 需要 <agent> <op> <paths...>\n\n{}",
                parts[0],
                Self::perm_usage()
            );
        }
        let agent = parts[1].to_owned();
        let op = parts[2].to_owned();
        let paths: Vec<String> = parts[3..].iter().map(|s| (*s).to_owned()).collect();
        let operation = if allow {
            closeclaw_common::PermissionOperation::AddFileWhitelist { agent, op, paths }
        } else {
            closeclaw_common::PermissionOperation::AddFileDeny { agent, op, paths }
        };
        self.handle_permission_op(&operation, sender_id).await
    }

    async fn handle_perm_cmd_op(
        &self,
        parts: &[&str],
        allow: bool,
        sender_id: Option<&str>,
    ) -> String {
        if parts.len() < 3 {
            return format!(
                "参数不足：{} 需要 <agent> <command> [args...]\n\n{}",
                parts[0],
                Self::perm_usage()
            );
        }
        let agent = parts[1].to_owned();
        let command = parts[2].to_owned();
        let cmd_args: Vec<String> = parts[3..].iter().map(|s| (*s).to_owned()).collect();
        let operation = if allow {
            closeclaw_common::PermissionOperation::AddCommandWhitelist {
                agent,
                command,
                args: cmd_args,
            }
        } else {
            closeclaw_common::PermissionOperation::AddCommandDeny {
                agent,
                command,
                args: cmd_args,
            }
        };
        self.handle_permission_op(&operation, sender_id).await
    }

    /// Handle `/user approve` directly in the Gateway.
    pub(crate) async fn handle_user_approve_cmd(
        &self,
        parts: &[&str],
        sender_id: Option<&str>,
    ) -> String {
        if parts.is_empty() {
            return format!(
                "参数不足：approve 需要 <request_id>\n\n{}",
                Self::user_usage()
            );
        }
        let request_id = parts[0].to_owned();
        let mut perms = vec![closeclaw_common::InitialPermissionSet::BasicMessaging];
        let mut i = 1;
        while i < parts.len() {
            if parts[i] == "--perms" {
                i += 1;
                if i >= parts.len() {
                    return format!(
                        "参数不足：--perms 需要一个集合名称\n\n{}",
                        Self::user_usage()
                    );
                }
                match Self::parse_perm_set(parts[i]) {
                    Some(p) => perms = vec![p],
                    None => return format!("无效的权限集合：{}。可选值：basic", parts[i]),
                }
            } else {
                return format!("未知参数：{}\n\n{}", parts[i], Self::user_usage());
            }
            i += 1;
        }
        self.handle_user_approve(&request_id, &perms, sender_id)
            .await
    }

    /// Handle `/user reject` directly in the Gateway.
    pub(crate) async fn handle_user_reject_cmd(
        &self,
        parts: &[&str],
        sender_id: Option<&str>,
    ) -> String {
        if parts.is_empty() {
            return format!(
                "参数不足：reject 需要 <request_id>\n\n{}",
                Self::user_usage()
            );
        }
        self.handle_user_reject(&parts[0], sender_id).await
    }

    pub(crate) fn user_usage() -> String {
        "用法：\n\
         /user list\n\
         /user approve <request_id> [--perms <set>]\n\
         /user reject <request_id>"
            .to_owned()
    }

    pub(crate) fn parse_perm_set(name: &str) -> Option<closeclaw_common::InitialPermissionSet> {
        match name.to_lowercase().as_str() {
            "basic" | "basic-messaging" => {
                Some(closeclaw_common::InitialPermissionSet::BasicMessaging)
            }
            _ => None,
        }
    }

    /// Check if a sender is a new unregistered user and auto-submit
    /// a user creation request via the ApprovalFlow.
    ///
    /// When a non-owner, unregistered user sends their first message:
    /// 1. Submit a user creation request via `ApprovalFlow::submit_user_creation()`
    /// 2. Notify the user that their request is pending approval
    /// 3. Return `Some(HandleResult::SlashHandled)` to block further processing
    ///
    /// Returns `None` if the sender is owner, already registered, or no
    /// approval flow is configured.
    pub(crate) async fn check_new_user_registration(
        &self,
        sender_id: &str,
        channel: &str,
    ) -> Option<HandleResult> {
        // Owner doesn't need registration.
        if sender_id == "owner" {
            return None;
        }

        // Load user registry from config_dir/users.json.
        let config_dir = self.get_config_dir().await?;
        let registry_path = config_dir.join("users.json");
        let registry: UserRegistry = tokio::fs::read_to_string(&registry_path)
            .await
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default();

        // Already registered → proceed normally.
        if registry.is_registered(sender_id) {
            return None;
        }

        // New user → submit creation request.
        let flow_guard = self.approval_flow.read().await;
        let Some(flow_arc) = flow_guard.as_ref() else {
            tracing::debug!(
                sender_id,
                "no approval flow configured, cannot register new user"
            );
            return None;
        };
        let mut flow = flow_arc.lock().await;
        match flow.submit_user_creation(sender_id, channel, vec![]) {
            Some(request_id) => {
                tracing::info!(
                    sender_id,
                    channel,
                    request_id = %request_id,
                    "new user registration request auto-submitted"
                );
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply(format!(
                        "👋 您是新用户，已向 Owner 提交注册申请（请求 ID: {}）。请等待审批。",
                        request_id
                    ))
                    .await;
                }
                Some(HandleResult::SlashHandled)
            }
            None => {
                // Duplicate request or other issue.
                tracing::debug!(
                    sender_id,
                    channel,
                    "user creation request already pending or failed"
                );
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply("⏳ 您的注册请求正在审批中，请等待。".to_owned())
                        .await;
                }
                Some(HandleResult::SlashHandled)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- is_path_dangerous ---

    #[test]
    fn test_path_dangerous_relative_traversal_unix() {
        assert!(Gateway::is_path_dangerous("../etc/passwd"));
    }

    #[test]
    fn test_path_dangerous_relative_traversal_windows() {
        assert!(Gateway::is_path_dangerous("..\\windows\\system32"));
    }

    #[test]
    fn test_path_dangerous_absolute_path() {
        assert!(Gateway::is_path_dangerous("/etc/passwd"));
    }

    #[test]
    fn test_path_dangerous_windows_drive() {
        assert!(Gateway::is_path_dangerous("C:\\Windows"));
    }

    #[test]
    fn test_path_dangerous_null_byte() {
        assert!(Gateway::is_path_dangerous("file\0etc/passwd"));
    }

    #[test]
    fn test_path_safe_relative() {
        assert!(!Gateway::is_path_dangerous("data/file.txt"));
    }

    #[test]
    fn test_path_safe_simple() {
        assert!(!Gateway::is_path_dangerous("file.txt"));
    }

    // --- parse_perm_set ---

    #[test]
    fn test_parse_perm_set_basic() {
        assert_eq!(
            Gateway::parse_perm_set("basic"),
            Some(closeclaw_common::InitialPermissionSet::BasicMessaging)
        );
    }

    #[test]
    fn test_parse_perm_set_basic_messaging() {
        assert_eq!(
            Gateway::parse_perm_set("basic-messaging"),
            Some(closeclaw_common::InitialPermissionSet::BasicMessaging)
        );
    }

    #[test]
    fn test_parse_perm_set_case_insensitive() {
        assert_eq!(
            Gateway::parse_perm_set("BASIC"),
            Some(closeclaw_common::InitialPermissionSet::BasicMessaging)
        );
    }

    #[test]
    fn test_parse_perm_set_unknown() {
        assert_eq!(Gateway::parse_perm_set("admin"), None);
    }

    #[test]
    fn test_parse_perm_set_empty() {
        assert_eq!(Gateway::parse_perm_set(""), None);
    }

    // --- perm_usage / user_usage ---

    #[test]
    fn test_perm_usage_contains_required_subcommands() {
        let usage = Gateway::perm_usage();
        assert!(usage.contains("allow-file"));
        assert!(usage.contains("deny-file"));
        assert!(usage.contains("allow-cmd"));
        assert!(usage.contains("deny-cmd"));
    }

    #[test]
    fn test_user_usage_contains_required_subcommands() {
        let usage = Gateway::user_usage();
        assert!(usage.contains("list"));
        assert!(usage.contains("approve"));
        assert!(usage.contains("reject"));
    }
}
