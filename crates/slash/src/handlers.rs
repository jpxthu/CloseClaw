//! Built-in slash command handlers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::{SlashHandler, SlashResult, SystemAppendAction};
use crate::registry::HandlerRegistry;
use closeclaw_common::build_git_status_for;
use closeclaw_gateway::SessionManager;
use closeclaw_session::persistence::ReasoningLevel;

// ── CompactHandler ──────────────────────────────────────────────────────────

/// `/compact` — manually trigger context compaction.
pub struct CompactHandler;

#[async_trait::async_trait]
impl SlashHandler for CompactHandler {
    fn commands(&self) -> &[&str] {
        &["compact"]
    }

    fn description(&self) -> &str {
        "手动压缩对话历史"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        let instruction = args.trim();
        if instruction.is_empty() {
            SlashResult::Compact { instruction: None }
        } else {
            SlashResult::Compact {
                instruction: Some(instruction.to_owned()),
            }
        }
    }
}

// ── ClearHandler ────────────────────────────────────────────────────────────

/// `/clear` — clear the system prompt static-layer cache and trigger rebuild.
pub struct ClearHandler {
    session_manager: Arc<SessionManager>,
}

impl ClearHandler {
    /// Create a new ClearHandler that operates on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for ClearHandler {
    fn commands(&self) -> &[&str] {
        &["clear"]
    }

    fn description(&self) -> &str {
        "清除 system prompt 静态层缓存并触发重建"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
        self.session_manager
            .rebuild_system_prompt(&ctx.session_id)
            .await;
        SlashResult::Reply("System prompt 缓存已清除，下次请求将重新构建。".to_owned())
    }
}

// ── HelpHandler ─────────────────────────────────────────────────────────────

/// `/help` — list all registered slash commands.
pub struct HelpHandler {
    registry: Arc<HandlerRegistry>,
}

impl HelpHandler {
    /// Create a new HelpHandler that reads command metadata from the given registry.
    pub fn new(registry: Arc<HandlerRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl SlashHandler for HelpHandler {
    fn commands(&self) -> &[&str] {
        &["help"]
    }

    fn description(&self) -> &str {
        "显示所有可用指令"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        let mut lines = vec!["可用指令：".to_owned()];
        let entries = self.registry.iter();
        let mut entries: Vec<(String, String)> = entries
            .into_iter()
            .map(|(cmd, h)| (cmd, h.description().to_owned()))
            .collect();
        // Sort by command name for stable output.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (cmd, desc) in entries {
            lines.push(format!("  /{cmd}  - {desc}"));
        }
        SlashResult::Reply(lines.join("\n"))
    }
}

// ── ReasoningHandler ───────────────────────────────────────────────────────

/// `/reasoning` — query or set the reasoning depth for the current session.
///
/// - No arguments: reply with the current reasoning level.
/// - With an argument (`low`, `medium`, `high`, `max`, `off`): update the
///   session's reasoning level via `SlashResult::SetReasoning`.
///
/// The `off` alias maps to `Low` (the minimum reasoning depth).
pub struct ReasoningHandler {
    session_manager: Arc<SessionManager>,
}

impl ReasoningHandler {
    /// Create a new ReasoningHandler operating on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }

    /// Parse a reasoning level string. Returns `None` for invalid values.
    fn parse_level(s: &str) -> Option<ReasoningLevel> {
        match s.to_lowercase().as_str() {
            "low" | "off" => Some(ReasoningLevel::Low),
            "medium" => Some(ReasoningLevel::Medium),
            "high" => Some(ReasoningLevel::High),
            "max" => Some(ReasoningLevel::Max),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl SlashHandler for ReasoningHandler {
    fn commands(&self) -> &[&str] {
        &["reasoning"]
    }

    fn description(&self) -> &str {
        "查询或设置推理深度"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let arg = args.trim();

        // No arguments — return the current reasoning level.
        if arg.is_empty() {
            let Some(conv) = self
                .session_manager
                .get_conversation_session(&ctx.session_id)
                .await
            else {
                return SlashResult::Reply("当前会话未激活".to_owned());
            };
            let cs = conv.read().await;
            let level = cs.reasoning_level();
            return SlashResult::Reply(format!("当前推理深度：{level}"));
        }

        // With argument — parse and return SetReasoning.
        match Self::parse_level(arg) {
            Some(level) => SlashResult::SetReasoning { level },
            None => SlashResult::Reply(format!(
                "无效的推理深度：{arg}。可选值：low, medium, high, max, off"
            )),
        }
    }
}

// ── ExecHandler ─────────────────────────────────────────────────────────────

/// `/exec` — execute a shell command as owner, gated by the permission engine.
///
/// `ExecHandler` itself does not perform any authorization: it only constructs
/// `SlashResult::Exec { command }`. The Gateway's permission routing is
/// responsible for evaluating the request via the permission engine before
/// execution.
pub struct ExecHandler;

#[async_trait::async_trait]
impl SlashHandler for ExecHandler {
    fn commands(&self) -> &[&str] {
        &["exec"]
    }

    fn description(&self) -> &str {
        "以 owner 身份执行 shell 命令（需权限审批）"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Exec {
            command: args.to_owned(),
        }
    }
}

// ── SystemHandler ──────────────────────────────────────────────────────────

/// `/system` — manage the system prompt append section.
///
/// - `/system add <content>` or `/system + <content>`: append an instruction.
/// - `/system list` or `/system` (no args): list current appended instructions.
/// - `/system clear`: clear all appended instructions.
pub struct SystemHandler {
    session_manager: Arc<SessionManager>,
}

impl SystemHandler {
    /// Create a new SystemHandler operating on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for SystemHandler {
    fn commands(&self) -> &[&str] {
        &["system"]
    }

    fn description(&self) -> &str {
        "管理 system prompt 追加区"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let parts: Vec<&str> = args.trim().splitn(2, |c: char| c.is_whitespace()).collect();
        let sub = parts[0];
        let remaining = if parts.len() > 1 { parts[1].trim() } else { "" };

        match sub {
            "add" | "+" => {
                if remaining.is_empty() {
                    SlashResult::Reply("用法：/system add <要追加的指令>".to_owned())
                } else {
                    SlashResult::SystemAppend {
                        action: SystemAppendAction::Add(remaining.to_owned()),
                    }
                }
            }
            "list" | "" => {
                let Some(conv) = self
                    .session_manager
                    .get_conversation_session(&ctx.session_id)
                    .await
                else {
                    return SlashResult::Reply("当前会话未激活".to_owned());
                };
                let cs = conv.read().await;
                let appends = cs.system_appends();
                if appends.is_empty() {
                    SlashResult::Reply("当前无追加指令".to_owned())
                } else {
                    let list: String = appends
                        .iter()
                        .enumerate()
                        .map(|(i, s)| format!("[{i}] {s}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    SlashResult::Reply(list)
                }
            }
            "clear" => SlashResult::SystemAppend {
                action: SystemAppendAction::Clear,
            },
            other => SlashResult::Reply(format!("未知子指令：{other}。支持 add / list / clear。")),
        }
    }
}

// ── WorkdirHandler ──────────────────────────────────────────────────────────

/// `/cd`, `/pwd`, `/git` — operate on the session's working directory.
///
/// All three commands are handled by a single struct because they share the
/// same `SessionManager` dependency. Subcommand dispatch happens inside
/// [`Self::handle`] by inspecting `ctx.command`.
///
/// - `/cd <path>`: validate the path exists, then look up the
///   `ConversationSession` via `SessionManager::get_conversation_session`,
///   mutate its workdir in place, and reply with the new path plus the
///   current git branch (if any).
/// - `/pwd`: read `ConversationSession::workdir` and reply with the path.
/// - `/git <args>`: placeholder reply — full implementation will route
///   through the permission engine for read-only git subcommands.
///
/// `immediate()` returns `false` so `/cd` queues behind a running LLM turn
/// (consistent with the design doc — workdir changes should not interrupt
/// in-flight LLM work).
pub struct WorkdirHandler {
    session_manager: Arc<SessionManager>,
}

impl WorkdirHandler {
    /// Create a new WorkdirHandler operating on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }

    /// Handle `/cd <path>`: validate path, set session workdir, reply with
    /// path + git status.
    async fn handle_cd(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let path_str = args.trim();
        if path_str.is_empty() {
            return SlashResult::Reply("用法：/cd <路径>".to_owned());
        }

        let path = PathBuf::from(path_str);
        if !Path::new(&path).exists() {
            return SlashResult::Reply(format!("目录不存在：{path_str}"));
        }

        // Inline: look up ConversationSession and mutate its workdir directly.
        // (Previously this lived on SessionManager::set_session_workdir, which
        // was a thin wrapper around the same two operations.)
        let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        else {
            return SlashResult::Reply("当前会话未激活".to_owned());
        };
        {
            let mut cs = conv.write().await;
            cs.set_workdir(path.clone());
        }

        let git_status = build_git_status_for(&path.to_string_lossy());
        let mut reply = format!("工作目录已变更为：{}", path.display());
        if let Some(status) = git_status {
            reply.push('\n');
            reply.push_str(&status);
        }
        SlashResult::Reply(reply)
    }

    /// Handle `/pwd`: read the current session workdir and reply with the path.
    async fn handle_pwd(&self, ctx: &SlashContext) -> SlashResult {
        let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        else {
            return SlashResult::Reply("当前会话未激活".to_owned());
        };
        let cs = conv.read().await;
        SlashResult::Reply(cs.workdir().display().to_string())
    }

    /// Handle `/git <args>`: placeholder. Real routing through the permission
    /// engine arrives in a follow-up step.
    fn handle_git(&self, _args: &str) -> SlashResult {
        SlashResult::Reply("git 指令即将支持".to_owned())
    }
}

#[async_trait::async_trait]
impl SlashHandler for WorkdirHandler {
    fn commands(&self) -> &[&str] {
        &["cd", "pwd", "git"]
    }

    fn description(&self) -> &str {
        "工作目录操作"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        match ctx.command.as_str() {
            "cd" => self.handle_cd(args, ctx).await,
            "pwd" => self.handle_pwd(ctx).await,
            "git" => self.handle_git(args),
            // WorkdirHandler should never be invoked with an unknown command;
            // the dispatcher only routes registered commands to us.
            other => SlashResult::Reply(format!(
                "未知子指令：{other}。WorkdirHandler 支持 cd / pwd / git。"
            )),
        }
    }
}
