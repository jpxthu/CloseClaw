//! `/bg` — manually trigger background execution for the current session.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_gateway::SessionManager;

/// `/bg` — move the currently running foreground command to background.
///
/// Calls [`SessionManager::trigger_manual_background`] to signal the session
/// that the current command should be backgrounded.
pub struct BackgroundHandler {
    session_manager: Arc<SessionManager>,
}

impl BackgroundHandler {
    /// Create a new BackgroundHandler operating on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for BackgroundHandler {
    fn commands(&self) -> &[&str] {
        &["bg"]
    }

    fn description(&self) -> &str {
        "将当前前台命令转为后台执行"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
        match self
            .session_manager
            .trigger_manual_background(&ctx.session_id)
            .await
        {
            Ok(true) => SlashResult::Reply("已将当前命令转为后台执行。".to_owned()),
            Ok(false) => SlashResult::Reply("当前没有需要后台化的命令。".to_owned()),
            Err(e) => SlashResult::Reply(format!("后台化失败：{e}")),
        }
    }
}
