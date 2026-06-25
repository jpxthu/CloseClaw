use crate::common::VerbosityLevel;
use crate::llm::session::ChatSession;
use crate::session::persistence::ReasoningLevel;
use crate::slash::context::SlashContext;
use crate::slash::side_effect::SideEffectContext;

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
                Self::execute_system_append(ctx, action).await;
            }
            SlashResult::NewSession => {
                Self::execute_new_session(ctx).await;
            }
            SlashResult::Stop => {
                Self::execute_stop(ctx).await;
            }
            SlashResult::SetMode(_) => {
                tracing::warn!("SlashResult::SetMode not yet routed through dispatch_slash");
            }
            SlashResult::Unknown(_) => {
                tracing::debug!("SlashResult::Unknown returned from handler");
            }
        }
    }

    async fn execute_system_append(ctx: &SideEffectContext, action: &SystemAppendAction) {
        match action {
            SystemAppendAction::Add(content) => {
                if let Some(cs) = ctx.get_conversation_session().await {
                    let mut session = cs.write().await;
                    let index = session.add_system_append(content.clone());
                    ctx.reply(format!("已追加指令（序号 {index}）")).await;
                } else {
                    ctx.reply("当前会话未激活，无法追加指令".to_owned()).await;
                }
            }
            SystemAppendAction::Clear => {
                if let Some(cs) = ctx.get_conversation_session().await {
                    let mut session = cs.write().await;
                    let count = session.clear_system_appends();
                    ctx.reply(format!("已清除 {count} 条追加指令")).await;
                } else {
                    ctx.reply("当前会话未激活，无法清除指令".to_owned()).await;
                }
            }
        }
    }

    async fn execute_new_session(ctx: &SideEffectContext) {
        let agent_id = ctx
            .session_manager
            .get_chat_id(&ctx.session_id)
            .await
            .unwrap_or_default();
        let new_session_id = ctx
            .session_manager
            .force_new_for_channel(&ctx.channel, &agent_id)
            .await;
        ctx.reply(format!("已创建新 session：{new_session_id}"))
            .await;
    }

    async fn execute_stop(ctx: &SideEffectContext) {
        let Some(conv) = ctx
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        else {
            ctx.reply("当前会话未激活".to_owned()).await;
            return;
        };
        let busy = {
            let cs = conv.read().await;
            cs.is_llm_busy()
        };
        if busy {
            let mut cs = conv.write().await;
            cs.cancel_token.cancel();
            let handles_to_stop: Vec<_> = {
                let child_handles = cs
                    .child_handles
                    .read()
                    .expect("child_handles lock poisoned");
                child_handles.values().filter_map(|w| w.upgrade()).collect()
            };
            cs.clear_pending();
            drop(cs);
            for child in handles_to_stop {
                let child_cs = child.read().await;
                child_cs.cancel_token.cancel();
            }
        }
        ctx.reply("已停止当前任务".to_owned()).await;
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
