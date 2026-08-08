//! Gateway-side implementation of [`SlashEffectExecutor`].
//!
//! Bridges the common trait to the Gateway's concrete
//! `SessionManager` and `SessionMessageHandler` for performing
//! slash command side effects. All trait and type definitions
//! (`ReplyAction`, `SideEffectContext`, `SlashEffectExecutor`,
//! `SlashResultExecutor`) are defined in `closeclaw_common::executor`.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::executor::{CompactionError, CompactionResult, SlashEffectExecutor};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::slash_router::SystemAppendAction;
use closeclaw_common::{ReasoningLevel, VerbosityLevel};
use closeclaw_session::llm_session::ConversationSession;
use tokio::sync::RwLock;

use super::session_manager::stop::GracefulStopOutcome;
use super::{SessionManager, SessionMessageHandler};

// ── SlashEffectExecutor implementation ──────────────────────────────────

/// Gateway-side implementation of [`SlashEffectExecutor`].
///
/// Bridges the common trait to the Gateway's concrete
/// `SessionManager` and `SessionMessageHandler` for performing
/// slash command side effects.
pub(crate) struct GatewaySlashExecutor {
    session_manager: Arc<SessionManager>,
    session_handler: Option<Arc<SessionMessageHandler>>,
}

#[async_trait]
impl SlashEffectExecutor for GatewaySlashExecutor {
    async fn execute_stop(&self, session_id: &str, cascade: bool, force: bool) {
        let mode = if force {
            ShutdownMode::Forceful
        } else {
            ShutdownMode::Graceful
        };
        let timeout = closeclaw_session::llm_session::session_handles::DEFAULT_GRACEFUL_TIMEOUT;
        let result = self
            .session_manager
            .stop_single_session(session_id, mode, cascade, timeout, None)
            .await;

        match result {
            Ok(GracefulStopOutcome::Completed) => {
                tracing::info!(
                    session_id = %session_id,
                    force = force,
                    cascade = cascade,
                    "session stopped successfully"
                );
            }
            Ok(GracefulStopOutcome::Interrupted) => {
                tracing::info!(
                    session_id = %session_id,
                    force = force,
                    cascade = cascade,
                    "session interrupted and force-stopped"
                );
            }
            Ok(GracefulStopOutcome::TimedOut { remaining, .. }) => {
                // User /stop: escalate to forceful on timeout.
                tracing::info!(
                    session_id = %session_id,
                    remaining = remaining,
                    "graceful timeout, escalating to forceful"
                );
                let force_result = self
                    .session_manager
                    .stop_single_session(
                        session_id,
                        ShutdownMode::Forceful,
                        cascade,
                        std::time::Duration::ZERO,
                        None,
                    )
                    .await;
                match force_result {
                    Ok(_) => {
                        tracing::info!(
                            session_id = %session_id,
                            "session force-stopped after timeout"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            session_id = %session_id,
                            error = ?e,
                            "force stop after timeout failed"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = ?e,
                    "stop failed"
                );
            }
        }
    }

    async fn execute_new_session(&self, _session_id: &str, channel: &str) -> String {
        // force_new_for_channel creates a fresh session for the channel and
        // updates the channel→session mapping so subsequent messages route to it.
        let agent_id = self
            .session_manager
            .get_chat_id(_session_id)
            .await
            .unwrap_or_default();
        self.session_manager
            .force_new_for_channel(channel, &agent_id)
            .await
    }

    async fn execute_compact(
        &self,
        session_id: &str,
        instruction: Option<String>,
    ) -> Result<CompactionResult, CompactionError> {
        let Some(sh) = self.session_handler.as_ref() else {
            return Err(CompactionError::HandlerNotAvailable(
                "session handler not available".to_string(),
            ));
        };

        // Build ChatFn: pure LLM forwarding layer.
        let fc = Arc::clone(&sh.fallback_client);
        let chat_fn = crate::session_handler_compact::build_chat_fn(fc);

        // Lock CompactionService and call SessionManager::compact.
        let mut svc = sh.compaction_service.lock().await;
        self.session_manager
            .compact(
                session_id,
                instruction.as_deref(),
                false,
                &mut svc,
                &chat_fn,
                None,
            )
            .await
    }

    async fn execute_system_append(&self, session_id: &str, action: &SystemAppendAction) -> usize {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        let Some(cs) = cs else {
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("session 不存在，无法执行系统指令".to_owned())
                    .await;
            }
            return 0;
        };
        // Create a PartialRewrite snapshot before modifying the system prompt,
        // per design doc: /system is a local rewrite that warrants a snapshot.
        let snapshot_id = self
            .session_manager
            .create_partial_rewrite_snapshot(session_id)
            .await;
        let count = {
            let mut cs = cs.write().await;
            let n = match action {
                SystemAppendAction::Add(text) => {
                    // add_system_append returns 0-based index; reply uses 1-based.
                    cs.add_system_append(text.clone()) + 1
                }
                SystemAppendAction::Clear => {
                    let n = cs.clear_system_appends();
                    // Invalidate static layer cache on clear, so the next
                    // prompt build regenerates from current state.
                    self.session_manager.invalidate_static_cache().await;
                    n
                }
            };
            // Mark snapshot as complete after successful modification.
            if let Some(ref sid) = snapshot_id {
                cs.mark_complete_snapshot(sid);
            }
            n
        };
        count
    }

    async fn execute_set_reasoning(&self, session_id: &str, level: ReasoningLevel) {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        let Some(cs) = cs else {
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("session 不存在，无法设置推理深度".to_owned())
                    .await;
            }
            return;
        };
        cs.write().await.set_reasoning_level(level);
    }

    async fn execute_set_verbosity(&self, session_id: &str, level: VerbosityLevel) {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        let Some(cs) = cs else {
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("session 不存在，无法设置输出详细度".to_owned())
                    .await;
            }
            return;
        };
        cs.write().await.set_verbosity_level(level);
    }

    async fn execute_set_mode(&self, session_id: &str, mode: &str) {
        let cs: Option<Arc<RwLock<ConversationSession>>> = self
            .session_manager
            .get_conversation_session(session_id)
            .await;
        let Some(cs) = cs else {
            if let Some(sh) = self.session_handler.as_ref() {
                sh.send_reply("session 不存在，无法设置 mode".to_owned())
                    .await;
            }
            return;
        };
        match closeclaw_common::SessionMode::from_str_opt(mode) {
            Some(parsed) => {
                cs.write().await.set_session_mode(parsed);
            }
            None => {
                tracing::warn!(
                    session_id,
                    mode,
                    "unknown session mode; keeping current mode"
                );
            }
        }
    }

    async fn execute_exec(
        &self,
        _session_id: &str,
        _agent_id: &str,
        command: &str,
    ) -> Vec<ContentBlock> {
        let command = command.trim();
        if command.is_empty() {
            return vec![ContentBlock::Text("用法：/exec <command>".to_owned())];
        }

        let parts: Vec<String> = shlex::split(command).unwrap_or_else(|| vec![command.to_owned()]);
        let cmd = parts.first().cloned().unwrap_or_default();
        let args = parts[1..].to_vec();

        // Permission is evaluated at the Gateway layer (check_slash_permission).
        // The executor layer no longer performs redundant permission checks.
        self.run_command(&cmd, &args).await
    }
}

// ── GatewaySlashExecutor inherent methods ──────────────────────────────

impl GatewaySlashExecutor {
    /// Create a new executor for testing purposes.
    #[allow(dead_code)]
    pub(crate) fn new(
        session_manager: Arc<SessionManager>,
        session_handler: Option<Arc<SessionMessageHandler>>,
    ) -> Self {
        Self {
            session_manager,
            session_handler,
        }
    }

    /// Execute a command and format stdout/stderr into ContentBlocks.
    async fn run_command(&self, cmd: &str, args: &[String]) -> Vec<ContentBlock> {
        let result = tokio::process::Command::new(cmd).args(args).output().await;
        match result {
            Ok(output) => {
                let mut blocks = Vec::new();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stdout.is_empty() {
                    blocks.push(ContentBlock::Text(stdout.to_string()));
                }
                if !stderr.is_empty() {
                    blocks.push(ContentBlock::Text(format!("[stderr] {stderr}")));
                }
                if blocks.is_empty() {
                    let code = output.status.code().unwrap_or(-1);
                    blocks.push(ContentBlock::Text(format!("命令执行完成，退出码：{code}")));
                }
                blocks
            }
            Err(e) => vec![ContentBlock::Text(format!("命令执行失败：{e}"))],
        }
    }
}
