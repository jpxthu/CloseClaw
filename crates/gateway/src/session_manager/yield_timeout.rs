//! Yield timeout protection for active Waiting sessions.
//!
//! When a session enters active Waiting via `sessions_yield`, a
//! configurable timeout timer is started. If child sessions do not
//! complete within the timeout, the timer fires:
//!
//! 1. All unfinished child sessions are terminated (cascade)
//! 2. A timeout notification is injected into the parent's message queue
//! 3. The session exits Waiting and resumes normal processing
//!
//! On normal recovery (all children completed), the timer is cancelled.

use std::sync::Arc;
use std::time::Duration;

use tracing::warn;

use super::stop::StopError;
use super::SessionManager;

/// Default yield timeout in seconds (10 minutes).
///
/// Used when neither the per-agent `subagents.timeout` nor the
/// spawn args specify a timeout.
const DEFAULT_YIELD_TIMEOUT_SECS: u64 = 600;

impl SessionManager {
    /// Start a yield timeout for the given session.
    ///
    /// Spawns a tokio task that waits for `timeout_secs` seconds.
    /// On expiry:
    /// 1. Terminates all child sessions via forceful stop
    /// 2. Injects a timeout notification into the parent's message queue
    /// 3. Exits Waiting state and triggers pending message drain
    ///
    /// If `timeout_secs` is `None`, uses [`DEFAULT_YIELD_TIMEOUT_SECS`].
    /// If a timeout is already running for this session, the old one is
    /// aborted first (defensive — callers should cancel before restarting).
    ///
    /// Takes `Arc<Self>` so the spawned task can hold a strong reference.
    pub async fn start_yield_timeout(
        self: &Arc<Self>,
        session_id: &str,
        agent_id: &str,
        timeout_secs: Option<u64>,
    ) {
        let secs = timeout_secs.unwrap_or(DEFAULT_YIELD_TIMEOUT_SECS);
        let duration = Duration::from_secs(secs);

        // Abort any existing timeout handle (defensive).
        self.cancel_yield_timeout(session_id).await;

        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent_id.to_string();
        let sm = Arc::clone(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            sm.handle_yield_timeout(&session_id_owned, &agent_id_owned, secs)
                .await;
        });

        self.yield_timeout_handles
            .write()
            .await
            .insert(session_id.to_string(), handle);
        tracing::info!(
            session_id = %session_id,
            timeout_secs = secs,
            "yield timeout started"
        );
    }

    /// Cancel the yield timeout for a session (normal recovery path).
    ///
    /// Called by `maybe_recover_yielded_session` when all children
    /// complete before the timeout expires.
    pub async fn cancel_yield_timeout(&self, session_id: &str) {
        if let Some(handle) = self.yield_timeout_handles.write().await.remove(session_id) {
            handle.abort();
            tracing::debug!(
                session_id = %session_id,
                "yield timeout cancelled"
            );
        }
    }

    /// Handle yield timeout expiry.
    ///
    /// Terminates all child sessions, injects a timeout notification,
    /// and resumes the parent session.
    async fn handle_yield_timeout(&self, session_id: &str, _agent_id: &str, timeout_secs: u64) {
        tracing::warn!(
            session_id = %session_id,
            "yield timeout fired: terminating child sessions"
        );

        // 1. Collect child session IDs from the SpawnTree.
        let child_ids: Vec<String> = {
            let children = self.children.read().await;
            children
                .list_children(session_id)
                .iter()
                .map(|info| info.session_id.clone())
                .collect()
        };

        // 2. Force-stop all child sessions.
        for child_id in &child_ids {
            match self
                .stop_single_session(
                    child_id,
                    closeclaw_common::shutdown::ShutdownMode::Forceful,
                    true,                      // cascade
                    std::time::Duration::ZERO, // not used in forceful mode
                    None,
                )
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        child_id = %child_id,
                        "yield timeout: child session terminated"
                    );
                }
                Err(StopError::Skipped) => {
                    tracing::debug!(
                        child_id = %child_id,
                        "yield timeout: child session already gone"
                    );
                }
                Err(e) => {
                    warn!(
                        child_id = %child_id,
                        error = ?e,
                        "yield timeout: failed to stop child session"
                    );
                }
            }
        }

        // 3. Inject timeout notification into the parent's message queue.
        if let Some(cs) = self.get_conversation_session(session_id).await {
            {
                let mut cs_write = cs.write().await;
                let notification = format!(
                    "[超时] 子 agent 任务在 {} 秒内未完成，已自动终止所有子 session。",
                    timeout_secs
                );
                cs_write.inject_system_message(notification);
            }
        }

        // 4. Exit Waiting state.
        if let Some(cs) = self.get_conversation_session(session_id).await {
            cs.read().await.exit_waiting();
        }

        // 5. Clean up the timeout handle entry.
        self.yield_timeout_handles.write().await.remove(session_id);

        // 6. Trigger pending message drain.
        self.drain_pending_for_session(session_id).await;

        tracing::info!(
            session_id = %session_id,
            "yield timeout handled: session resumed"
        );
    }
}
