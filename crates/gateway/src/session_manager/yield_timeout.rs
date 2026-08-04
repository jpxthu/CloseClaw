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
//! Child sessions are NOT force-terminated — per-child spawn timeouts
//! handle individual child termination. The yield timeout only
//! notifies the parent and resumes it.
//!
//! On normal recovery (all children completed), the timer is cancelled.

use std::sync::Arc;
use std::time::Duration;

use super::SessionManager;
use closeclaw_common::ChildSessionState;
use closeclaw_tasks::NotificationPriority;

impl SessionManager {
    /// Start a yield timeout for the given session.
    ///
    /// Spawns two tokio tasks:
    /// - **Warning timer**: Injects a warning notification (next priority)
    ///   60 seconds before the hard timeout fires, giving the agent early
    ///   visibility into slow child sessions. Skipped when
    ///   `overall_timeout_secs <= 60`.
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
    ) {
        let duration = Duration::from_secs(overall_timeout_secs);
        let warning_secs = if overall_timeout_secs > 60 {
            overall_timeout_secs - 60
        } else {
            0
        };
        let warning_duration = Duration::from_secs(warning_secs);

        // Abort any existing timeout handles (defensive).
        self.cancel_yield_timeout(session_id).await;

        // Spawn warning timer (fires first, only if > 60s).
        if warning_secs > 0 {
            let session_id_warn = session_id.to_string();
            let agent_id_warn = agent_id.to_string();
            let sm_warn = Arc::clone(self);
            let warning_handle = tokio::spawn(async move {
                tokio::time::sleep(warning_duration).await;
                sm_warn
                    .handle_yield_warning(&session_id_warn, &agent_id_warn, warning_secs)
                    .await;
            });
            self.yield_warning_handles
                .write()
                .await
                .insert(session_id.to_string(), warning_handle);
        }

        // Spawn hard timeout timer.
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

        tracing::info!(
            session_id = %session_id,
            timeout_secs = overall_timeout_secs,
            warning_secs = warning_secs,
            "yield timeout started (warning + hard)"
        );
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
    async fn handle_yield_warning(&self, session_id: &str, _agent_id: &str, warning_secs: u64) {
        tracing::warn!(
            session_id = %session_id,
            "yield warning timeout fired: injecting warning notification"
        );

        if let Some(cs) = self.get_conversation_session(session_id).await {
            let mut cs_write = cs.write().await;
            let notification = format!(
                "[⚠️ 超时预警] 子 agent 任务已运行 {} 秒，即将到达超时上限。\n请耐心等待或检查子 session 状态。",
                warning_secs
            );
            cs_write.push_system_notification(notification, NotificationPriority::Next);
        }

        // Clean up the warning handle entry (the task is done).
        self.yield_warning_handles.write().await.remove(session_id);

        tracing::info!(
            session_id = %session_id,
            "yield warning notification injected"
        );
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

        // 1. Collect child info with states and elapsed times.
        let child_summaries = {
            let children = self.children.read().await;
            let child_list = children.list_children(session_id);

            if child_list.is_empty() {
                "(无子 session)".to_string()
            } else {
                let mut summaries = Vec::new();
                for info in child_list {
                    let elapsed = info.created_at.elapsed();
                    let elapsed_secs = elapsed.as_secs();
                    let status_str = self.child_status_str(session_id, &info.session_id).await;
                    summaries.push(format!(
                        "  - {} [{}] 已运行 {} 秒",
                        info.session_id, status_str, elapsed_secs
                    ));
                }
                summaries.join("\n")
            }
        };

        // 2. Push structured timeout notification.
        if let Some(cs) = self.get_conversation_session(session_id).await {
            let mut cs_write = cs.write().await;
            let notification = format!(
                "[超时] 父 session 等待上限 {} 秒已到。\n\n子 session 状态:\n{}\n\n仍在运行的子 session 将继续执行，完成后结果按正常路径注入。",
                timeout_secs,
                child_summaries
            );
            cs_write.push_system_notification(notification, NotificationPriority::Next);
        }

        // 3. Exit Waiting state.
        if let Some(cs) = self.get_conversation_session(session_id).await {
            cs.read().await.exit_waiting();
        }

        // 4. Clean up the timeout handle entry.
        self.yield_timeout_handles.write().await.remove(session_id);

        // 5. Trigger pending message drain.
        self.drain_pending_for_session(session_id).await;

        tracing::info!(
            session_id = %session_id,
            "yield timeout handled: session resumed"
        );
    }

    /// Look up a child session's status string from the parent's
    /// `child_states` map.
    async fn child_status_str(&self, parent_id: &str, child_id: &str) -> String {
        if let Some(cs) = self.get_conversation_session(parent_id).await {
            let guard = cs.read().await;
            let states = guard
                .child_states
                .read()
                .expect("child_states lock poisoned");
            if let Some((state, _)) = states.get(child_id) {
                match state {
                    ChildSessionState::Running => "运行中",
                    ChildSessionState::Completed => "已完成",
                    ChildSessionState::Terminated => "已终止",
                    ChildSessionState::Errored => "出错",
                }
                .to_string()
            } else {
                "未知".to_string()
            }
        } else {
            "未知".to_string()
        }
    }
}
