//! Yield timeout protection for active Waiting sessions.
//!
//! When a session enters active Waiting via `sessions_yield`, a
//! configurable timeout timer is started. If child sessions do not
//! complete within the timeout, the timer fires:
//!
//! 1. A structured timeout notification is injected (listing each
//!    child session's ID, status, and elapsed time)
//! 2. The session exits Waiting and resumes normal processing
//!
//! **Warning modes** (injected before hard timeout):
//! - *Cyclic*: When `timeout_warning_secs = Some(ws)`, warnings begin
//!   starting after `ws` seconds of execution, repeating every
//!   `ws * ratio` seconds until hard timeout fires.
//! - *Legacy*: When `timeout_warning_secs = None`, a single warning
//!   fires 60 seconds before hard timeout.
//!
//! Child sessions are NOT force-terminated — per-child spawn timeouts
//! handle individual child termination. The yield timeout only
//! notifies the parent and resumes it.
//!
//! On normal recovery (all children completed), the timer is cancelled.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::SessionManager;
use closeclaw_common::ChildSessionState;
use closeclaw_session::compaction::{
    estimate_total_tokens, get_context_window, CompactConfig, CompactionMessage,
};
use closeclaw_session::llm_session::{ChatSession, SessionMessage};
use closeclaw_session::persistence::PendingOperationDetail;
use closeclaw_session::spawn::ChildSessionInfo;
use closeclaw_tasks::NotificationPriority;

impl SessionManager {
    /// Start a yield timeout for the given session.
    ///
    /// Spawns two tokio tasks:
    /// - **Warning timer** (two modes):
    ///   - *Cyclic*: When `timeout_warning_secs` is `Some(ws)` and
    ///     `ws < overall_timeout_secs`, cyclic warnings begin starting
    ///     after `ws` seconds of execution, repeating every
    ///     `ws * ratio` seconds until the hard timeout fires.
    ///   - *Legacy*: When `timeout_warning_secs` is `None`, a single
    ///     warning fires 60 seconds before the hard timeout.
    /// - **Hard timeout** (`overall_timeout_secs`): Injects a structured
    ///   timeout notification and resumes the parent.
    ///
    /// If a timeout is already running for this session, the old ones are
    /// aborted first (defensive — callers should cancel before restarting).
    ///
    /// Takes `Arc<Self>` so the spawned task can hold a strong reference.
    pub async fn start_yield_timeout(
        self: &Arc<Self>,
        session_id: &str,
        agent_id: &str,
        overall_timeout_secs: u64,
        timeout_warning_secs: Option<u64>,
        notify_interval_ratio: Option<f64>,
    ) {
        let duration = Duration::from_secs(overall_timeout_secs);

        // Abort any existing timeout handles (defensive).
        self.cancel_yield_timeout(session_id).await;

        // Determine warning mode: cyclic (ws) or legacy (None).
        let effective_warning = match timeout_warning_secs {
            Some(ws) if ws < overall_timeout_secs => Some(ws),
            Some(ws) => {
                tracing::warn!(
                    session_id = %session_id,
                    timeout_warning_secs = ws,
                    overall_timeout_secs = overall_timeout_secs,
                    "timeout_warning_secs >= overall_timeout_secs, \
                     falling back to legacy single warning"
                );
                None
            }
            None => None,
        };
        if let Some(warning_secs) = effective_warning {
            self.spawn_cyclic_warning(
                session_id,
                agent_id,
                overall_timeout_secs,
                warning_secs,
                notify_interval_ratio,
            )
            .await;
        } else {
            self.spawn_legacy_warning(session_id, agent_id, overall_timeout_secs)
                .await;
        }

        // Spawn hard timeout timer (warning handles cleaned up by
        // cancel_yield_timeout or handle_yield_timeout).
        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent_id.to_string();
        let sm = Arc::clone(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            sm.handle_yield_timeout(&session_id_owned, &agent_id_owned, overall_timeout_secs)
                .await;
        });
        self.yield_timeout_handles
            .write()
            .await
            .insert(session_id.to_string(), handle);
    }

    /// Cancel the yield timeout for a session (normal recovery path).
    ///
    /// Cancels both the hard timeout and the warning timeout handles.
    /// Called by `maybe_recover_yielded_session` when all children
    /// complete before the timeout expires.
    pub async fn cancel_yield_timeout(&self, session_id: &str) {
        if let Some(handle) = self.yield_timeout_handles.write().await.remove(session_id) {
            handle.abort();
        }
        if let Some(handle) = self.yield_warning_handles.write().await.remove(session_id) {
            handle.abort();
        }
        tracing::debug!(
            session_id = %session_id,
            "yield timeout cancelled"
        );
    }

    /// Handle yield warning timeout expiry.
    ///
    /// Injects a warning notification (next priority) into the parent's
    /// message queue. Children continue executing — they are not terminated.
    ///
    /// For cyclic mode, `elapsed` is seconds since overall timeout start,
    /// and `overall_timeout_secs` is the hard timeout, so the notification
    /// can show remaining time.
    ///
    /// Notification content follows design-doc §超时预警（timeout_warning）→
    /// 通知内容:
    /// 1. 设定的预期执行时长（warning_secs）
    /// 2. 实际运行时长（per-child from created_at）
    /// 3. 硬超时时间及剩余时间
    /// 4. context window 使用情况（已用/总容量）
    /// 5. 当前 token 用量（prompt + completion）
    async fn handle_yield_warning(
        &self,
        session_id: &str,
        _agent_id: &str,
        elapsed: u64,
        overall_timeout_secs: u64,
        warning_secs: Option<u64>,
    ) {
        let remaining = overall_timeout_secs.saturating_sub(elapsed);
        tracing::warn!(
            session_id = %session_id,
            elapsed_secs = elapsed,
            remaining_secs = remaining,
            "yield warning timeout fired: injecting warning notification"
        );

        // Build per-child summaries (items 2, 4, 5).
        let child_summaries = self.build_warning_child_summaries(session_id).await;

        if let Some(cs) = self.get_conversation_session(session_id).await {
            let mut cs_write = cs.write().await;

            // Item 1: timeout_warning_secs设定值.
            let warning_line = match warning_secs {
                Some(ws) => format!("设定预期执行时长: {} 秒", ws),
                None => "设定预期执行时长: 未设定（legacy 模式）".to_string(),
            };

            // Item 3: 硬超时时间及剩余时间.
            let timeout_line = format!(
                "硬超时时间: {} 秒 | 剩余: {} 秒",
                overall_timeout_secs, remaining
            );

            let notification = format!(
                "[⚠️ 超时预警] 子 agent 任务已运行 {} 秒。\n\n\
                 \u{251c}\u{2500} {}\n\
                 \u{251c}\u{2500} {}\n\
                 \u{251c}\u{2500} 请耐心等待或检查子 session 状态。\n\
                 \u{2514}\u{2500} 子 session 详情:\n{}",
                elapsed, warning_line, timeout_line, child_summaries
            );
            cs_write.push_system_notification(notification, NotificationPriority::Next);
        }

        tracing::info!(
            session_id = %session_id,
            "yield warning notification injected"
        );
    }

    /// Spawn a cyclic warning task that sends warnings every `interval`
    /// seconds starting at `warning_secs` elapsed time, until the hard
    /// timeout fires.
    async fn spawn_cyclic_warning(
        self: &Arc<Self>,
        session_id: &str,
        agent_id: &str,
        overall_timeout_secs: u64,
        warning_secs: u64,
        notify_interval_ratio: Option<f64>,
    ) {
        let ratio = notify_interval_ratio.unwrap_or(0.5).clamp(0.1, 2.0);
        let interval_secs = (warning_secs as f64 * ratio).round() as u64;
        let interval_secs = interval_secs.max(1);
        let warning_duration = Duration::from_secs(warning_secs);
        let interval_duration = Duration::from_secs(interval_secs);

        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent_id.to_string();
        let sm = Arc::clone(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(warning_duration).await;
            let mut elapsed = warning_secs;
            loop {
                sm.handle_yield_warning(
                    &session_id_owned,
                    &agent_id_owned,
                    elapsed,
                    overall_timeout_secs,
                    Some(warning_secs),
                )
                .await;
                elapsed = elapsed.saturating_add(interval_secs);
                if elapsed >= overall_timeout_secs {
                    break;
                }
                tokio::time::sleep(interval_duration).await;
            }
            sm.yield_warning_handles
                .write()
                .await
                .remove(&session_id_owned);
        });
        self.yield_warning_handles
            .write()
            .await
            .insert(session_id.to_string(), handle);

        tracing::info!(
            session_id = %session_id,
            timeout_secs = overall_timeout_secs,
            warning_secs = warning_secs,
            interval_secs = interval_secs,
            "yield timeout started (cyclic warning + hard)"
        );
    }

    /// Spawn a legacy single-warning task that fires 60 seconds before
    /// the hard timeout. Only emits one warning notification.
    async fn spawn_legacy_warning(
        self: &Arc<Self>,
        session_id: &str,
        agent_id: &str,
        overall_timeout_secs: u64,
    ) {
        let warning_secs = overall_timeout_secs.saturating_sub(60);
        if warning_secs == 0 {
            tracing::info!(
                session_id = %session_id,
                timeout_secs = overall_timeout_secs,
                "yield timeout started (legacy single warning skipped, too short)"
            );
            return;
        }

        let warning_duration = Duration::from_secs(warning_secs);
        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent_id.to_string();
        let sm = Arc::clone(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(warning_duration).await;
            sm.handle_yield_warning(
                &session_id_owned,
                &agent_id_owned,
                warning_secs,
                overall_timeout_secs,
                None,
            )
            .await;
        });
        self.yield_warning_handles
            .write()
            .await
            .insert(session_id.to_string(), handle);

        tracing::info!(
            session_id = %session_id,
            timeout_secs = overall_timeout_secs,
            warning_secs = warning_secs,
            "yield timeout started (legacy single warning + hard)"
        );
    }

    /// Build per-child warning summaries for the yield warning notification.
    ///
    /// Returns a formatted string with each child's:
    /// - Actual running time (from created_at)
    /// - Context window usage (estimated tokens / total capacity)
    /// - Token usage (prompt + completion)
    ///
    /// Returns "(无子 session)" when no children exist.
    async fn build_warning_child_summaries(&self, session_id: &str) -> String {
        let children = self.children.read().await;
        let child_list = children.list_children(session_id);

        if child_list.is_empty() {
            return "  (无子 session)".to_string();
        }

        let mut summaries = Vec::new();
        for info in child_list {
            let lines = self.build_single_child_warning(&info).await;
            summaries.push(lines.join("\n"));
        }
        summaries.join("\n")
    }

    /// Build warning lines for a single child session.
    async fn build_single_child_warning(&self, info: &ChildSessionInfo) -> Vec<String> {
        let elapsed_secs = info.created_at.elapsed().as_secs();
        let mut lines = vec![format!(
            "  - {} [已运行 {} 秒]",
            info.session_id, elapsed_secs
        )];

        if let Some(child_cs) = self.get_conversation_session(&info.session_id).await {
            let cs_read = child_cs.read().await;
            let model = cs_read.model().to_string();
            let stats = cs_read.stats().clone();
            let messages = ChatSession::messages(&*cs_read);

            let compaction_msgs = Self::build_compaction_messages(messages);
            let chars_per_token = CompactConfig::default().chars_per_token;
            let estimated_tokens = estimate_total_tokens(&stats, &compaction_msgs, chars_per_token);
            let context_window = get_context_window(&model, None);
            lines.push(format!(
                "    context window: {} / {} tokens",
                estimated_tokens, context_window
            ));

            lines.push(format!(
                "    token 用量: prompt={} completion={}",
                stats.total_prompt_tokens, stats.total_completion_tokens
            ));
        }
        lines
    }

    /// Convert chat messages into CompactionMessages for token
    /// estimation.
    fn build_compaction_messages(messages: &[SessionMessage]) -> Vec<CompactionMessage> {
        messages
            .iter()
            .filter(|m| m.role == "user" || m.role == "assistant")
            .map(|m| CompactionMessage {
                role: m.role.clone(),
                content: m
                    .content_blocks
                    .iter()
                    .map(|b| match b {
                        closeclaw_llm::types::ContentBlock::Text(t) => t.as_str(),
                        closeclaw_llm::types::ContentBlock::Thinking { thinking: t, .. } => {
                            t.as_str()
                        }
                        closeclaw_llm::types::ContentBlock::ToolUse { input, .. } => input.as_str(),
                        closeclaw_llm::types::ContentBlock::ToolResult { content, .. } => {
                            content.as_str()
                        }
                        _ => "",
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect()
    }

    /// Build a human-readable summary of child sessions for the timeout
    /// notification. Each child is listed with its status and elapsed time.
    async fn build_child_summaries(
        &self,
        session_id: &str,
        child_states_map: &HashMap<String, (ChildSessionState, Option<PendingOperationDetail>)>,
    ) -> String {
        let children = self.children.read().await;
        let child_list = children.list_children(session_id);

        if child_list.is_empty() {
            return "(无子 session)".to_string();
        }

        let mut summaries = Vec::new();
        for info in child_list {
            let elapsed_secs = info.created_at.elapsed().as_secs();
            let status_str = child_states_map
                .get(&info.session_id)
                .map(|(state, _)| match state {
                    ChildSessionState::Running => "运行中",
                    ChildSessionState::Completed => "已完成",
                    ChildSessionState::Terminated => "已终止",
                    ChildSessionState::Errored => "出错",
                })
                .unwrap_or("未知");
            summaries.push(format!(
                "  - {} [{}] 已运行 {} 秒",
                info.session_id, status_str, elapsed_secs
            ));
        }
        summaries.join("\n")
    }

    /// Handle yield hard timeout expiry.
    ///
    /// Builds a structured notification listing each child session's
    /// ID, status (completed/running), and elapsed time, then injects
    /// it and resumes the parent. Child sessions are NOT force-terminated
    /// — per-child spawn timeouts handle individual termination.
    async fn handle_yield_timeout(&self, session_id: &str, _agent_id: &str, timeout_secs: u64) {
        tracing::warn!(
            session_id = %session_id,
            "yield timeout fired: injecting structured notification"
        );

        // 1. Collect child info (hoist lookup outside loop).
        let parent_cs = self.get_conversation_session(session_id).await;
        let child_states_map = if let Some(ref cs) = parent_cs {
            let guard = cs.read().await;
            let map = guard
                .child_states
                .read()
                .expect("child_states lock poisoned")
                .clone();
            map
        } else {
            return;
        };

        let child_summaries = self
            .build_child_summaries(session_id, &child_states_map)
            .await;

        // 2. Push structured timeout notification.
        if let Some(cs) = &parent_cs {
            let mut cs_write = cs.write().await;
            let notification = format!(
                "[超时] 父 session 等待上限 {} 秒已到。\n\n\
                 子 session 状态:\n{}\n\n\
                 仍在运行的子 session 将继续执行，\
                 完成后结果按正常路径注入。",
                timeout_secs, child_summaries
            );
            cs_write.push_system_notification(notification, NotificationPriority::Next);
        }

        // 3. Exit Waiting state.
        if let Some(cs) = &parent_cs {
            cs.read().await.exit_waiting();
        }

        // 4. Clean up the timeout handle entry.
        self.yield_timeout_handles.write().await.remove(session_id);
        self.yield_warning_handles.write().await.remove(session_id);

        // 5. Trigger pending message drain.
        self.drain_pending_for_session(session_id).await;

        tracing::info!(
            session_id = %session_id,
            "yield timeout handled: session resumed"
        );
    }
}
