//! Helper functions for [`GatewaySlashExecutor`](super::GatewaySlashExecutor).
//!
//! Extracted from `slash_permission.rs` to keep individual files under the
//! 800-line soft limit. Each function is `pub(crate)` so the executor impl
//! in the parent module can call them.

use std::sync::Arc;

use closeclaw_common::processor::ContentBlock;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::slash_router::SystemAppendAction;
use closeclaw_session::llm_session::ConversationSession;
use tokio::sync::RwLock;

use super::session_manager::stop::{GracefulStopOutcome, StopOptions};
use super::{SessionManager, SessionMessageHandler};

/// Stop the current LLM turn, escalating to forceful on timeout.
pub(crate) async fn gw_stop(sm: &SessionManager, session_id: &str, cascade: bool, force: bool) {
    let mode = if force {
        ShutdownMode::Forceful
    } else {
        ShutdownMode::Graceful
    };
    let timeout = closeclaw_session::llm_session::session_handles::DEFAULT_GRACEFUL_TIMEOUT;
    let result = sm
        .stop_single_session(
            session_id,
            mode,
            cascade,
            StopOptions {
                timeout,
                progress_tx: None,
                clear_queue: true,
            },
        )
        .await;

    match result {
        Ok(GracefulStopOutcome::Completed) => {
            tracing::info!(session_id, force, cascade, "session stopped successfully");
        }
        Ok(GracefulStopOutcome::Interrupted) => {
            tracing::info!(
                session_id,
                force,
                cascade,
                "session interrupted and force-stopped"
            );
        }
        Ok(GracefulStopOutcome::TimedOut { remaining, .. }) => {
            tracing::info!(
                session_id,
                remaining,
                "graceful timeout, escalating to forceful"
            );
            let force_result = sm
                .stop_single_session(
                    session_id,
                    ShutdownMode::Forceful,
                    cascade,
                    StopOptions {
                        timeout: std::time::Duration::ZERO,
                        progress_tx: None,
                        clear_queue: true,
                    },
                )
                .await;
            match force_result {
                Ok(_) => tracing::info!(session_id, "session force-stopped after timeout"),
                Err(e) => tracing::warn!(session_id, error = ?e, "force stop after timeout failed"),
            }
        }
        Err(e) => {
            tracing::warn!(session_id, error = ?e, "stop failed");
        }
    }
}

/// Trigger context compaction via the session handler.
pub(crate) async fn gw_compact(
    sm: &SessionManager,
    sh: &Option<Arc<SessionMessageHandler>>,
    session_id: &str,
    instruction: Option<String>,
) -> Result<
    closeclaw_session::compaction::CompactionResult,
    closeclaw_session::compaction::CompactionError,
> {
    let Some(sh) = sh.as_ref() else {
        return Err(
            closeclaw_session::compaction::CompactionError::HandlerNotAvailable(
                "session handler not available".to_string(),
            ),
        );
    };
    let fc = Arc::clone(&sh.fallback_client);
    let chat_fn = crate::session_handler_compact::build_chat_fn(fc);
    let mut svc = sh.compaction_service.lock().await;
    let result = sm
        .compact(
            session_id,
            instruction.as_deref(),
            false,
            &mut svc,
            &chat_fn,
            None,
        )
        .await;
    if result.is_ok() {
        sh.reset_circuit_breaker_notification();
    }
    result
}

/// Apply a system prompt append/clear action.
pub(crate) async fn gw_system_append(
    sm: &SessionManager,
    sh: &Option<Arc<SessionMessageHandler>>,
    session_id: &str,
    action: &SystemAppendAction,
) -> usize {
    let cs: Option<Arc<RwLock<ConversationSession>>> =
        sm.get_conversation_session(session_id).await;
    let Some(cs) = cs else {
        if let Some(sh) = sh.as_ref() {
            sh.send_reply("session 不存在，无法执行系统指令".to_owned())
                .await;
        }
        return 0;
    };
    let snapshot_id = sm.create_partial_rewrite_snapshot(session_id).await;
    let count = {
        let mut cs = cs.write().await;
        let n = match action {
            SystemAppendAction::Add(text) => cs.add_system_append(text.clone()) + 1,
            SystemAppendAction::Clear => {
                let n = cs.clear_system_appends();
                sm.invalidate_static_cache().await;
                n
            }
        };
        if let Some(ref sid) = snapshot_id {
            cs.mark_complete_snapshot(sid);
        }
        n
    };
    count
}

/// Get the conversation session or send an error reply.
pub(crate) async fn gw_get_cs_or_reply(
    sm: &SessionManager,
    sh: &Option<Arc<SessionMessageHandler>>,
    session_id: &str,
    error_msg: &str,
) -> Option<Arc<RwLock<ConversationSession>>> {
    let cs: Option<Arc<RwLock<ConversationSession>>> =
        sm.get_conversation_session(session_id).await;
    if cs.is_none() {
        if let Some(sh) = sh.as_ref() {
            sh.send_reply(error_msg.to_owned()).await;
        }
    }
    cs
}

/// Create a new session for the given channel.
pub(crate) async fn gw_new_session(sm: &SessionManager, session_id: &str, channel: &str) -> String {
    let agent_id = sm.get_chat_id(session_id).await.unwrap_or_default();
    sm.force_new_for_channel(channel, &agent_id).await
}

/// Parse and execute a shell command.
pub(crate) async fn gw_exec(command: &str) -> Vec<ContentBlock> {
    let command = command.trim();
    if command.is_empty() {
        return vec![ContentBlock::Text("用法：/exec <command>".to_owned())];
    }
    let parts: Vec<String> = shlex::split(command).unwrap_or_else(|| vec![command.to_owned()]);
    let cmd = parts.first().cloned().unwrap_or_default();
    let args = parts[1..].to_vec();
    // Permission is evaluated at the Gateway layer (check_slash_permission).
    run_command(&cmd, &args).await
}

/// Execute a command and format stdout/stderr into ContentBlocks.
async fn run_command(cmd: &str, args: &[String]) -> Vec<ContentBlock> {
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
