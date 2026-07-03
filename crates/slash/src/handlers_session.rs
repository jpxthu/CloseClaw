//! Session-related slash command handlers.
//!
//! Extracted from `handlers.rs` to keep individual files under 500 lines.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::VerbosityLevel;
use closeclaw_gateway::SessionManager;
use closeclaw_llm::session::{ChatSession, ConversationSession};

// ── NewSessionHandler ────────────────────────────────────────────────────

/// `/new` — create a new session for the current channel.
///
/// The Gateway routes `SlashResult::NewSession` by calling
/// `SessionManager::force_new_for_channel`, which creates a fresh
/// `ConversationSession` and updates the channel→session mapping.
/// The old session is preserved in the sessions map for recovery.
pub struct NewSessionHandler;

#[async_trait::async_trait]
impl SlashHandler for NewSessionHandler {
    fn commands(&self) -> &[&str] {
        &["new"]
    }

    fn description(&self) -> &str {
        "创建新会话"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::NewSession
    }
}

// ── StopHandler ───────────────────────────────────────────────────────────

/// `/stop` — terminate the current running task.
///
/// The Gateway routes `SlashResult::Stop` by cancelling the active LLM turn,
/// cascading to child handles, and clearing the pending message queue.
pub struct StopHandler;

#[async_trait::async_trait]
impl SlashHandler for StopHandler {
    fn commands(&self) -> &[&str] {
        &["stop"]
    }

    fn description(&self) -> &str {
        "终止当前运行的任务"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Stop
    }
}

// ── VerboseHandler ──────────────────────────────────────────────────────────

/// `/verbose` — query or set the verbosity level for the current session.
///
/// - No arguments: reply with the current verbosity level.
/// - With an argument (`full`, `normal`, `off`): update the session's verbosity
///   level via `SlashResult::SetVerbosity`.
pub struct VerboseHandler {
    session_manager: Arc<SessionManager>,
}

impl VerboseHandler {
    /// Create a new VerboseHandler operating on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }

    /// Parse a verbosity level string. Returns `None` for invalid values.
    fn parse_level(s: &str) -> Option<VerbosityLevel> {
        match s.to_lowercase().as_str() {
            "full" => Some(VerbosityLevel::Full),
            "normal" => Some(VerbosityLevel::Normal),
            "off" => Some(VerbosityLevel::Off),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl SlashHandler for VerboseHandler {
    fn commands(&self) -> &[&str] {
        &["verbose"]
    }

    fn description(&self) -> &str {
        "查询或设置输出详细度"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let arg = args.trim();

        // No arguments — return the current verbosity level.
        if arg.is_empty() {
            let Some(conv) = self
                .session_manager
                .get_conversation_session(&ctx.session_id)
                .await
            else {
                return SlashResult::Reply("当前会话未激活".to_owned());
            };
            let cs = conv.read().await;
            let level = cs.verbosity_level();
            return SlashResult::Reply(format!("当前输出详细度：{level}"));
        }

        // With argument — parse and return SetVerbosity.
        match Self::parse_level(arg) {
            Some(level) => SlashResult::SetVerbosity { level },
            None => SlashResult::Reply(format!(
                "无效的输出详细度：{arg}。可选值：full, normal, off"
            )),
        }
    }
}

// ── StatusHandler ──────────────────────────────────────────────────────

/// `/status` — display the current session status.
///
/// Reads various fields from the [`ConversationSession`] and formats them
/// into a human-readable status report.
pub struct StatusHandler {
    session_manager: Arc<SessionManager>,
}

impl StatusHandler {
    /// Create a new StatusHandler operating on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for StatusHandler {
    fn commands(&self) -> &[&str] {
        &["status"]
    }

    fn description(&self) -> &str {
        "查看当前会话状态"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
        let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        else {
            return SlashResult::Reply("当前会话未激活".to_owned());
        };
        let cs = conv.read().await;

        let busy = cs.is_llm_busy();
        let llm_status = if busy { "运行中" } else { "空闲" };

        let model = cs.model();
        let reasoning = cs.reasoning_level();
        let total_tokens = cs.stats().total_tokens;

        // Count active child handles (Weak references that are still alive).
        let child_handles = cs.child_handles.read().unwrap_or_else(|e| e.into_inner());
        let active_children: usize = child_handles
            .values()
            .filter(
                |w: &&std::sync::Weak<tokio::sync::RwLock<ConversationSession>>| {
                    w.upgrade().is_some()
                },
            )
            .count();

        let workdir = cs.workdir().display();
        let appends = cs.system_appends();

        // TODO: 「当前模式」字段暂不可用（需 mode 基础设施）

        let mut lines = vec![
            format!("LLM 状态：{llm_status}"),
            format!("模型：{model}"),
            format!("推理深度：{reasoning}"),
            format!("上下文用量：{total_tokens} tokens"),
            format!("活跃子 agent：{active_children}"),
            format!("工作目录：{workdir}"),
        ];

        if appends.is_empty() {
            lines.push("追加指令：无".to_owned());
        } else {
            lines.push("追加指令：".to_owned());
            for (i, s) in appends.iter().enumerate() {
                lines.push(format!("  [{i}] {s}"));
            }
        }

        SlashResult::Reply(lines.join("\n"))
    }
}
