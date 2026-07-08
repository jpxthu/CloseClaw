//! `/perm` — permission management slash command handler.
//!
//! Routes `/perm` subcommands to [`PermissionOperation`] variants:
//! - `/perm allow-file <agent> <op> <paths...>`
//! - `/perm deny-file <agent> <op> <paths...>`
//! - `/perm allow-cmd <agent> <command> [args...]`
//! - `/perm deny-cmd <agent> <command> [args...]`

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::permission_op::PermissionOperation;
use closeclaw_common::slash_router::SlashResult;

/// `/perm` — manage permission rules (Owner only).
///
/// Parses subcommands and returns [`SlashResult::PermissionOp`] for the
/// gateway to execute. Invalid or incomplete arguments produce a
/// [`SlashResult::Reply`] with usage guidance.
pub struct PermissionSlashHandler;

impl PermissionSlashHandler {
    /// Usage text shown on errors or bare `/perm`.
    fn usage() -> String {
        "用法：\n\
         /perm allow-file <agent> <op> <paths...>\n\
         /perm deny-file <agent> <op> <paths...>\n\
         /perm allow-cmd <agent> <command> [args...]\n\
         /perm deny-cmd <agent> <command> [args...]"
            .to_owned()
    }

    /// Parse `/perm <subcmd> <args>` and return the appropriate result.
    fn dispatch(args: &str) -> SlashResult {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.is_empty() {
            return SlashResult::Reply(Self::usage());
        }

        match parts[0] {
            "allow-file" => Self::parse_file_op(parts, true),
            "deny-file" => Self::parse_file_op(parts, false),
            "allow-cmd" => Self::parse_cmd_op(parts, true),
            "deny-cmd" => Self::parse_cmd_op(parts, false),
            other => SlashResult::Reply(format!("未知子命令：{other}\n\n{}", Self::usage())),
        }
    }

    /// Parse a file permission subcommand (allow-file / deny-file).
    ///
    /// Expected: `<subcmd> <agent> <op> <paths...>`
    fn parse_file_op(parts: &[&str], allow: bool) -> SlashResult {
        if parts.len() < 4 {
            return SlashResult::Reply(format!(
                "参数不足：{} 需要 <agent> <op> <paths...>\n\n{}",
                parts[0],
                Self::usage()
            ));
        }
        let agent = parts[1].to_owned();
        let op = parts[2].to_owned();
        let paths: Vec<String> = parts[3..].iter().map(|s| (*s).to_owned()).collect();

        let operation = if allow {
            PermissionOperation::AddFileWhitelist { agent, op, paths }
        } else {
            PermissionOperation::AddFileDeny { agent, op, paths }
        };

        SlashResult::PermissionOp { op: operation }
    }

    /// Parse a command permission subcommand (allow-cmd / deny-cmd).
    ///
    /// Expected: `<subcmd> <agent> <command> [args...]`
    fn parse_cmd_op(parts: &[&str], allow: bool) -> SlashResult {
        if parts.len() < 3 {
            return SlashResult::Reply(format!(
                "参数不足：{} 需要 <agent> <command> [args...]\n\n{}",
                parts[0],
                Self::usage()
            ));
        }
        let agent = parts[1].to_owned();
        let command = parts[2].to_owned();
        let cmd_args: Vec<String> = parts[3..].iter().map(|s| (*s).to_owned()).collect();

        let operation = if allow {
            PermissionOperation::AddCommandWhitelist {
                agent,
                command,
                args: cmd_args,
            }
        } else {
            PermissionOperation::AddCommandDeny {
                agent,
                command,
                args: cmd_args,
            }
        };

        SlashResult::PermissionOp { op: operation }
    }
}

#[async_trait::async_trait]
impl SlashHandler for PermissionSlashHandler {
    fn commands(&self) -> &[&str] {
        &["perm"]
    }

    fn description(&self) -> &str {
        "管理权限规则（Owner only）"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        Self::dispatch(args.trim())
    }
}
