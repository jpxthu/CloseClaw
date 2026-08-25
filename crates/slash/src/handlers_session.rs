//! Session-related slash command handlers.
//!
//! Extracted from `handlers.rs` to keep individual files under 500 lines.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::SlashSessionQuery;
use closeclaw_common::VerbosityLevel;

// ── NewSessionHandler ────────────────────────────────────────────────────

/// `/new` — create a new session for the current channel.
///
/// The Gateway routes `SlashResult::NewSession` by calling
/// `SessionManager::force_new_for_channel`, which creates a fresh
/// `ConversationSession` and updates the channel→session mapping.
/// The old session is preserved in the sessions map for recovery.
#[derive(Clone)]
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

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
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
#[derive(Clone)]
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

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        // No arguments, no flags — always Forceful semantics per design doc.
        SlashResult::Stop {
            cascade: true,
            force: true,
        }
    }
}

// ── VerboseHandler ──────────────────────────────────────────────────────────

/// `/verbose` — query or set the verbosity level for the current session.
///
/// - No arguments: reply with the current verbosity level.
/// - With an argument (`full`, `normal`, `off`): update the session's verbosity
///   level via `SlashResult::SetVerbosity`.
#[derive(Clone)]
pub struct VerboseHandler {
    session_manager: Arc<dyn SlashSessionQuery>,
}

impl VerboseHandler {
    /// Create a new VerboseHandler operating on the given session manager.
    pub fn new(session_manager: Arc<dyn SlashSessionQuery>) -> Self {
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

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let arg = args.trim();

        // No arguments — return the current verbosity level.
        if arg.is_empty() {
            let Some(level_str) = self
                .session_manager
                .get_verbosity_level(&ctx.session_id)
                .await
            else {
                return SlashResult::Reply("当前会话未激活".to_owned());
            };
            return SlashResult::Reply(format!("当前输出详细度：{level_str}"));
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
#[derive(Clone)]
pub struct StatusHandler {
    session_manager: Arc<dyn SlashSessionQuery>,
}

impl StatusHandler {
    /// Create a new StatusHandler operating on the given session manager.
    pub fn new(session_manager: Arc<dyn SlashSessionQuery>) -> Self {
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
        let sm = &self.session_manager;
        let sid = &ctx.session_id;

        // Check if session exists.
        if sm.get_session_mode(sid).await.is_none() {
            return SlashResult::Reply("当前会话未激活".to_owned());
        }

        let busy = sm.is_llm_busy(sid).await;
        let llm_status = if busy { "运行中" } else { "空闲" };

        let model = sm.get_model(sid).await.unwrap_or_default();
        let reasoning = sm.get_reasoning_level(sid).await.unwrap_or_default();
        let mode_label = sm.get_session_mode(sid).await.unwrap_or_default();

        let (total_tokens, prompt_tokens, cache_read, cache_write) =
            sm.get_stats(sid).await.unwrap_or((0, 0, 0, 0));
        let cache_hit_rate = if prompt_tokens == 0 {
            "N/A".to_owned()
        } else {
            format!("{:.1}%", cache_read as f64 / prompt_tokens as f64 * 100.0)
        };

        let active_children = sm.get_active_child_count(sid).await;
        let workdir = sm.get_workdir(sid).await;
        let workdir_str = workdir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let appends = sm.get_system_appends(sid).await;

        let mut lines = vec![
            format!("LLM 状态：{llm_status}"),
            format!("模型：{model}"),
            format!("推理深度：{reasoning}"),
            format!("当前模式：{mode_label}"),
            format!("上下文用量：{total_tokens} tokens"),
            format!("缓存命中率：{cache_hit_rate}"),
            format!("缓存读 token：{cache_read}"),
            format!("缓存写 token：{cache_write}"),
            format!("活跃子 agent：{active_children}"),
            format!("工作目录：{workdir_str}"),
        ];

        if appends.is_empty() {
            lines.push("追加指令：无".to_owned());
        } else {
            lines.push("追加指令：".to_owned());
            for (i, s) in appends.iter().enumerate() {
                lines.push(format!("  [{i}] {s}"));
            }
        }

        // Cache break event (most recent).
        if let Some(cb) = sm.get_last_cache_break(sid).await {
            lines.push(cb);
        }

        SlashResult::Reply(lines.join("\n"))
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
}
