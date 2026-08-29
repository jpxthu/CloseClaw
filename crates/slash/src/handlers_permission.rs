//! `/perm` — permission management slash command handler.
//!
//! Routes `/perm` subcommands to permission operations.
//! These are now intercepted by the Gateway before reaching this handler,
//! but the handler is kept as a fallback documentation of the command format.
//!
//! Gateway intercepts:
//! - `/perm allow-file <agent> <op> <paths...>`
//! - `/perm deny-file <agent> <op> <paths...>`
//! - `/perm allow-cmd <agent> <command> [args...]`
//! - `/perm deny-cmd <agent> <command> [args...]`

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::slash_router::SlashResult;

/// `/perm` — manage permission rules (Owner only).
///
/// Permission commands are intercepted by the Gateway before reaching this
/// handler. The handler returns a Reply with usage information as a fallback.
#[derive(Clone)]
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
    ///
    /// # Panics
    /// All permission subcommands are intercepted by the Gateway before
    /// reaching this handler. This method exists only for test invocations
    /// and should never be reached in production.
    fn dispatch(args: &str) -> SlashResult {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.is_empty() {
            return SlashResult::Reply(Self::usage());
        }

        match parts[0] {
            "allow-file" => {
                unreachable!("/perm allow-file is intercepted by Gateway")
            }
            "deny-file" => {
                unreachable!("/perm deny-file is intercepted by Gateway")
            }
            "allow-cmd" => {
                unreachable!("/perm allow-cmd is intercepted by Gateway")
            }
            "deny-cmd" => {
                unreachable!("/perm deny-cmd is intercepted by Gateway")
            }
            other => SlashResult::Reply(format!("未知子命令：{other}\n\n{}", Self::usage())),
        }
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

    fn immediate(&self, _cmd: &str, _args: &str) -> bool {
        true
    }

    fn requires_permission(&self) -> bool {
        false
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        // In production, Gateway intercepts /perm before reaching this handler.
        // Subcommands are unreachable; only bare `/perm` returns usage.
        Self::dispatch(args.trim())
    }
}
