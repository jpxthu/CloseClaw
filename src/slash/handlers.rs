//! Built-in slash command handlers.

use std::sync::Arc;

use crate::gateway::SessionManager;
use crate::slash::context::SlashContext;
use crate::slash::handler::{SlashHandler, SlashResult};
use crate::slash::registry::HandlerRegistry;

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

    fn immediate(&self) -> bool {
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

    fn immediate(&self) -> bool {
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

    fn immediate(&self) -> bool {
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
