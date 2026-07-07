//! Mode-related slash command handlers.
//!
//! `/plan` enters Plan Mode; `/mode` queries or switches session mode.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_gateway::SessionManager;
use closeclaw_session::plan_file;

// ── PlanModeHandler ───────────────────────────────────────────────────────

/// `/plan` — enter Plan Mode with an optional task description.
///
/// - With arguments: creates a plan file in the session's workdir,
///   returns `SlashResult::SetMode` with the plan file path.
/// - Without arguments: replies with a usage hint.
pub struct PlanModeHandler {
    session_manager: Arc<SessionManager>,
}

impl PlanModeHandler {
    /// Create a new PlanModeHandler with access to session state.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for PlanModeHandler {
    fn commands(&self) -> &[&str] {
        &["plan"]
    }

    fn description(&self) -> &str {
        "进入 Plan Mode"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        if args.trim().is_empty() {
            return SlashResult::Reply(
                "用法：/plan <任务描述>\n进入 Plan Mode 进行任务规划。".to_owned(),
            );
        }

        let title = args.trim();
        let plan_file_path = if let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        {
            let cs = conv.read().await;
            let workdir = cs.workdir().to_path_buf();
            plan_file::create_plan_file(&workdir, title).ok()
        } else {
            None
        };

        SlashResult::SetMode {
            mode: "plan".to_owned(),
            plan_file_path,
        }
    }
}

// ── ModeHandler ──────────────────────────────────────────────────────────

/// `/mode` — query or switch the session mode.
///
/// - No arguments: reads the current `SessionMode` and replies.
/// - With an argument (`normal`, `plan`, `auto`): returns
///   `SlashResult::SetMode` to trigger the mode switch.
pub struct ModeHandler {
    session_manager: Arc<SessionManager>,
}

impl ModeHandler {
    /// Create a new ModeHandler operating on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for ModeHandler {
    fn commands(&self) -> &[&str] {
        &["mode"]
    }

    fn description(&self) -> &str {
        "查询或切换会话模式"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let arg = args.trim();

        // No arguments — return the current session mode.
        if arg.is_empty() {
            let Some(conv) = self
                .session_manager
                .get_conversation_session(&ctx.session_id)
                .await
            else {
                return SlashResult::Reply("当前会话未激活".to_owned());
            };
            let cs = conv.read().await;
            let mode = cs.session_mode();
            return SlashResult::Reply(format!("当前会话模式：{mode}"));
        }

        // With argument — validate and return SetMode.
        match SessionMode::from_str_opt(arg) {
            Some(mode) => SlashResult::SetMode {
                mode: mode.to_string(),
                plan_file_path: None,
            },
            None => {
                SlashResult::Reply(format!("无效的会话模式：{arg}。可选值：normal, plan, auto"))
            }
        }
    }
}
