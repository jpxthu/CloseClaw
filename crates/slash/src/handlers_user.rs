//! `/user` — user registration management slash command handler.
//!
//! Routes `/user` subcommands:
//! - `/user list` → list all registered users
//! - `/user approve <request_id> [--perms <set>]` → approve registration
//! - `/user reject <request_id>` → reject registration

use std::path::PathBuf;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::permission_op::InitialPermissionSet;
use closeclaw_common::slash_router::SlashResult;

/// `/user` — manage user registration (Owner only).
///
/// Parses subcommands and returns the appropriate [`SlashResult`].
///
/// Note: `/user approve` and `/user reject` are now intercepted by the
/// Gateway before reaching this handler. The handler remains as a fallback
/// for `/user list` and for documentation purposes.
pub struct UserSlashHandler {
    /// Config directory for reading `users.json`.
    config_dir: PathBuf,
}

impl UserSlashHandler {
    /// Create a new handler rooted at the given config directory.
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Usage text shown on errors or bare `/user`.
    fn usage() -> String {
        "用法：\n\
         /user list\n\
         /user approve <request_id> [--perms <set>]\n\
         /user reject <request_id>"
            .to_owned()
    }

    /// List all registered users from `users.json`.
    async fn handle_list(&self) -> SlashResult {
        let path = self.config_dir.join("users.json");
        let data = match tokio::fs::read_to_string(&path).await {
            Ok(d) => d,
            Err(_) => return SlashResult::Reply("暂无已注册用户".to_owned()),
        };
        let registry: closeclaw_permission::UserRegistry = match serde_json::from_str(&data) {
            Ok(r) => r,
            Err(_) => return SlashResult::Reply("解析用户列表失败".to_owned()),
        };
        let users = registry.list_users();
        if users.is_empty() {
            return SlashResult::Reply("暂无已注册用户".to_owned());
        }
        let mut lines = vec![format!("已注册用户（{}）：", users.len())];
        for u in users {
            let perms: Vec<&str> = u
                .initial_permissions
                .iter()
                .map(|p: &InitialPermissionSet| p.label())
                .collect();
            lines.push(format!(
                "  {} | 渠道: {} | 权限: [{}] | 注册于: {}",
                u.user_id,
                u.im_channel,
                perms.join(", "),
                u.created_at,
            ));
        }
        SlashResult::Reply(lines.join("\n"))
    }

    /// Parse `/user approve <request_id> [--perms <set>]`.
    fn handle_approve(args: &str) -> SlashResult {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.is_empty() {
            return SlashResult::Reply(format!(
                "参数不足：approve 需要 <request_id>\n\n{}",
                Self::usage()
            ));
        }
        SlashResult::Reply(format!(
            "✅ 用户注册审批请求已转发至 Gateway（请求 ID: {}）",
            parts[0]
        ))
    }

    /// Parse `/user reject <request_id>`.
    fn handle_reject(args: &str) -> SlashResult {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.is_empty() {
            return SlashResult::Reply(format!(
                "参数不足：reject 需要 <request_id>\n\n{}",
                Self::usage()
            ));
        }
        SlashResult::Reply(format!(
            "✅ 用户注册拒绝请求已转发至 Gateway（请求 ID: {}）",
            parts[0]
        ))
    }
}

#[async_trait::async_trait]
impl SlashHandler for UserSlashHandler {
    fn commands(&self) -> &[&str] {
        &["user"]
    }

    fn description(&self) -> &str {
        "管理用户注册（Owner only）"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        let args = args.trim();
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.is_empty() {
            return SlashResult::Reply(Self::usage());
        }
        match parts[0] {
            "list" => self.handle_list().await,
            "approve" => Self::handle_approve(args.trim_start_matches("approve").trim()),
            "reject" => Self::handle_reject(args.trim_start_matches("reject").trim()),
            other => SlashResult::Reply(format!("未知子命令：{other}\n\n{}", Self::usage())),
        }
    }
}
