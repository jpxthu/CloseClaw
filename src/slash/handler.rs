use crate::common::VerbosityLevel;
use crate::slash::context::SlashContext;
use crate::slash::side_effect::SideEffectContext;
use closeclaw_session::persistence::ReasoningLevel;

/// Action for the `SystemAppend` slash result.
#[derive(Debug, Clone)]
pub enum SystemAppendAction {
    /// Append a new system prompt instruction.
    Add(String),
    /// Clear all appended system prompt instructions.
    Clear,
}

/// Result of a slash command dispatch.
#[derive(Debug)]
pub enum SlashResult {
    /// Reply with a text message.
    Reply(String),
    /// Switch mode (future).
    SetMode(String),
    /// Create a new session (future).
    NewSession,
    /// Stop the current run (future).
    Stop,
    /// Compact context with optional instruction.
    Compact { instruction: Option<String> },
    /// Append to system prompt (future).
    SystemAppend { action: SystemAppendAction },
    /// Execute a sub-command (future).
    Exec { command: String },
    /// Set the reasoning level for the current session.
    SetReasoning { level: ReasoningLevel },
    /// Set the verbosity level for the current session.
    SetVerbosity { level: VerbosityLevel },
    /// Unknown command — no handler matched.
    Unknown(String),
}

impl SlashResult {
    /// Execute the side effects for this slash result variant.
    ///
    /// Side effects are communicated through `ctx`'s reply channel.
    /// The Gateway collects these actions and dispatches them.
    pub async fn execute(&self, ctx: &SideEffectContext) {
        match self {
            SlashResult::Reply(text) => {
                ctx.reply(text.clone()).await;
            }
            SlashResult::Compact { instruction } => {
                ctx.trigger_compact(instruction.clone()).await;
            }
            SlashResult::Exec { command } => {
                ctx.reply(format!("命令已提交审批：/{command}")).await;
            }
            SlashResult::SetReasoning { level } => {
                if let Some(cs) = ctx.get_conversation_session().await {
                    cs.write().await.set_reasoning_level(*level);
                    ctx.reply(format!("推理深度已设置为 {:?}", level)).await;
                } else {
                    ctx.reply("当前会话未激活，无法设置推理深度".to_owned())
                        .await;
                }
            }
            SlashResult::SetVerbosity { level } => {
                if let Some(cs) = ctx.get_conversation_session().await {
                    cs.write().await.set_verbosity_level(*level);
                    ctx.reply(format!("输出详细度已设置为 {level}")).await;
                } else {
                    ctx.reply("当前会话未激活，无法设置输出详细度".to_owned())
                        .await;
                }
            }
            SlashResult::SystemAppend { action } => {
                crate::slash::side_effect::execute_system_append(ctx, action).await;
            }
            SlashResult::NewSession => {
                crate::slash::side_effect::execute_new_session(ctx).await;
            }
            SlashResult::Stop => {
                crate::slash::side_effect::execute_stop(ctx).await;
            }
            SlashResult::SetMode(_) => {
                tracing::warn!("SlashResult::SetMode not yet routed through dispatch_slash");
            }
            SlashResult::Unknown(_) => {
                tracing::debug!("SlashResult::Unknown returned from handler");
            }
        }
    }
}

/// Trait for slash command handlers.
///
/// Implementors define which commands they handle, a description for help
/// text, whether the command is immediate (responds even when LLM is busy),
/// and the async execution logic.
#[async_trait::async_trait]
pub trait SlashHandler: Send + Sync {
    /// Command names (without the leading `/`).
    fn commands(&self) -> &[&str];

    /// Short description (for /help listing).
    fn description(&self) -> &str;

    /// Whether this is an immediate command (responds even when LLM is busy).
    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    /// Whether this command requires permission evaluation by the permission
    /// engine before execution. High-risk handlers (e.g. `/exec`) override this
    /// to return `true`; the default is `false` (safe handlers execute directly).
    fn requires_permission(&self) -> bool {
        false
    }

    /// Execute the command with the given arguments and context.
    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult;
}
