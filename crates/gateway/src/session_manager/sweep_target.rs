//! [`AnnounceSweepTarget`] trait implementation for [`SessionManager`].
//!
//! Migrated from `announce.rs` in Step 1.2 to keep the file under
//! the 1000-line hard limit. This module also adds the three new
//! trait methods introduced for stale-child detection:
//!
//! - [`get_last_output_at`] — read the child's last activity timestamp.
//! - [`is_parent_archived`] — check if the parent session is not active.
//! - [`terminate_stale_child`] — kill a stale child and notify the parent.

use super::SessionManager;
use closeclaw_common::{ChildCompletionStatus, SessionExecStatus};
use closeclaw_session::run_health::AnnounceSweepTarget;
use closeclaw_tasks::NotificationPriority;
use tracing::warn;

use super::announce::build_announce_event;
use super::spawn::SpawnMode;
use super::spawn_reclaim_gc::sweep_spawn_tree_reclaim;

/// Threshold in seconds for stale-child detection.
/// Must match `STALE_CHILD_THRESHOLD_SECS` in
/// `closeclaw-session::run_health::announce_sweeper`.
const STALE_CHILD_THRESHOLD_SECS: u64 = 300;

// ── Existing methods (pure migration from announce.rs) ─────────────────────

#[async_trait::async_trait]
impl AnnounceSweepTarget for SessionManager {
    async fn get_run_mode_children(&self) -> Vec<(String, String)> {
        let tree = self.children.read().await;
        tree.iter()
            .flat_map(|(_parent, infos)| infos.iter())
            .filter(|info| info.mode == SpawnMode::Run)
            .map(|info| (info.session_id.clone(), info.parent_session_id.clone()))
            .collect()
    }

    async fn is_child_removed(&self, child_id: &str) -> bool {
        let tree = self.children.read().await;
        tree.find_child(child_id).is_none()
    }

    async fn is_session_idle(&self, session_id: &str) -> bool {
        let Some(child_cs) = self.get_conversation_session(session_id).await else {
            // Session not found in memory — treat as not idle.
            return false;
        };
        let cs = child_cs.read().await;
        matches!(cs.exec_status(), SessionExecStatus::Idle)
    }

    async fn try_push_announce(&self, session_id: &str, priority: NotificationPriority) {
        SessionManager::try_push_announce(self, session_id, priority).await;
    }

    // ── New methods for stale-child detection ─────────────────────────────

    async fn get_last_output_at(&self, session_id: &str) -> Option<i64> {
        let cs = self.get_conversation_session(session_id).await?;
        let guard = cs.read().await;
        Some(guard.last_activity_at())
    }

    async fn is_parent_archived(&self, parent_id: &str) -> bool {
        let sessions = self.sessions.read().await;
        !sessions.contains_key(parent_id)
    }

    async fn terminate_stale_child(&self, parent_id: &str, child_id: &str) {
        if let Err(e) = self.kill_child(parent_id, child_id).await {
            warn!(
                parent_id = %parent_id,
                child_id = %child_id,
                error = %e,
                "sweep_target: kill_child failed for stale child"
            );
            return;
        }

        // Skip notification if parent session is archived.
        if self.is_parent_archived(parent_id).await {
            return;
        }

        let event = build_announce_event(
            child_id,
            String::new(),
            format!(
                "子 agent 已僵死（超过 {} 秒无新产出），已被自动终止",
                STALE_CHILD_THRESHOLD_SECS
            ),
            NotificationPriority::Next,
            ChildCompletionStatus::Terminated,
        );

        if let Err(e) = self.push_announce(parent_id, event).await {
            warn!(
                parent_id = %parent_id,
                error = %e,
                "sweep_target: push_announce for stale notification failed"
            );
        }
    }

    async fn sweep_reclaim(&self) {
        sweep_spawn_tree_reclaim(self).await;
    }
}
