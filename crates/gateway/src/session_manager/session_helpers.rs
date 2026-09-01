//! Helper functions extracted from SessionManager::find_or_create
//! to keep the main file under the 500-line limit.

use crate::Message;
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::persistence::{PersistenceService, SessionStatus};
use closeclaw_session::workspace;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

/// Generate a unique session ID.
///
/// Format: `{agent_id}_{YYYYMMDDhhmmss}_{8-hex}`
pub(super) fn generate_session_id(agent_id: &str) -> String {
    let ts = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let uuid = Uuid::new_v4();
    let hex_part = format!("{:08x}", uuid.as_fields().0);
    format!("{}_{}_{}", agent_id, ts, hex_part)
}

/// Compute the workdir path for a new session.
pub(super) async fn compute_session_workdir(
    restored: bool,
    session_id: &str,
    message: &Message,
    workspace_dir: &Option<PathBuf>,
    cm: &CheckpointManager<dyn PersistenceService>,
) -> Result<PathBuf, closeclaw_common::processor::ProcessError> {
    if restored {
        let checkpoint = cm.load(session_id).await.ok().flatten();
        let aid = checkpoint
            .as_ref()
            .and_then(|cp| cp.agent_id.clone())
            .unwrap_or_else(|| message.to.clone());
        let uid = checkpoint
            .as_ref()
            .and_then(|cp| cp.sender_id.clone())
            .unwrap_or_else(|| message.from.clone());
        if let Some(ref wd) = workspace_dir {
            Ok(workspace::ensure_workspace_dir(wd, &aid, &uid)
                .unwrap_or_else(|_| PathBuf::from("/tmp")))
        } else {
            Ok(PathBuf::from("/tmp"))
        }
    } else if let Some(ref workspace_dir) = workspace_dir {
        workspace::ensure_workspace_dir(workspace_dir, &message.to, &message.from).map_err(|e| {
            closeclaw_common::processor::ProcessError::ChainFailed(format!(
                "workspace creation failed: {}",
                e
            ))
        })
    } else {
        Ok(PathBuf::from("/tmp"))
    }
}

/// Create a Session entry from checkpoint data (no Message needed).
///
/// Used by startup recovery injection where no inbound message is available.
pub(super) fn create_session_from_checkpoint(session_id: &str, agent_id: &str) -> crate::Session {
    crate::Session {
        id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        channel: String::new(),
        created_at: chrono::Utc::now().timestamp(),
        depth: 0,
    }
}

/// Create and persist a brand-new session.
pub(super) fn create_new_session(
    session_id: &str,
    message: &Message,
    channel: &str,
) -> crate::Session {
    crate::Session {
        id: session_id.to_string(),
        agent_id: message.to.clone(),
        channel: channel.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        depth: 0,
    }
}

/// Update thread_id and reply_ref fields in a session's checkpoint.
///
/// Loads the checkpoint from storage, overwrites `thread_id` and
/// `reply_ref` with the provided values, and saves it back.
///
/// Silently skips (with warn!) when:
/// - storage is not available
/// - checkpoint does not exist
pub(super) async fn update_checkpoint_fields(
    cm: &CheckpointManager<dyn PersistenceService>,
    session_id: &str,
    thread_id: &Option<String>,
    reply_ref: &Option<String>,
) {
    let mut cp = match cm.load(session_id).await {
        Ok(Some(cp)) => cp,
        Ok(None) | Err(_) => {
            warn!(session_id = %session_id, "checkpoint not found, skipping thread_id/reply_ref update");
            return;
        }
    };
    cp.thread_id = thread_id.clone();
    cp.reply_ref = reply_ref.clone();
    if let Err(e) = cm.save_raw(&cp).await {
        warn!(session_id = %session_id, error = %e, "failed to save checkpoint with updated thread_id");
    }
}

/// Update `last_user_activity_at` and `last_message_at` for a user message.
///
/// Loads the checkpoint from storage, sets both timestamps to now,
/// and saves it back. Only called for user messages (not LLM responses,
/// tool results, or system messages).
///
/// Silently skips (with warn!) when:
/// - storage is not available
/// - checkpoint does not exist
pub(super) async fn update_checkpoint_user_activity(
    cm: &CheckpointManager<dyn PersistenceService>,
    session_id: &str,
) {
    let mut cp = match cm.load(session_id).await {
        Ok(Some(cp)) => cp,
        Ok(None) | Err(_) => {
            warn!(
                session_id = %session_id,
                "checkpoint not found, skipping user activity update"
            );
            return;
        }
    };
    let now = chrono::Utc::now();
    cp.last_user_activity_at = Some(now);
    cp.last_message_at = Some(now);
    if let Err(e) = cm.save_raw(&cp).await {
        warn!(
            session_id = %session_id,
            error = %e,
            "failed to save checkpoint with updated user activity"
        );
    }
}

/// Result of attempting to restore an archived session.
pub(super) struct RestoreResult {
    /// Whether the restoration was attempted and succeeded.
    pub restored: bool,
    /// The chat_id to send the restore notification to (via Gateway outbound chain).
    /// `None` if no notification is needed (e.g. not archived or restore failed).
    pub notification_chat_id: Option<String>,
}

/// Attempt to restore an archived session.
///
/// Returns a [`RestoreResult`] indicating whether restoration succeeded and,
/// if so, the chat_id to which the restore notification should be sent via
/// Gateway's outbound chain (instead of sending directly via IMPlugin).
pub(super) async fn try_restore_archived_session_inner(
    storage: &Arc<dyn PersistenceService>,
    session_id: &str,
    _channel: &str,
) -> RestoreResult {
    // Try active/migrating first, then fall back to archived storage.
    let checkpoint = match storage.load_checkpoint(session_id).await {
        Ok(Some(cp)) => Some(cp),
        _ => None,
    };
    let checkpoint = match checkpoint {
        Some(cp) => cp,
        None => match storage.load_archived_checkpoint(session_id).await {
            Ok(Some(cp)) => cp,
            _ => {
                return RestoreResult {
                    restored: false,
                    notification_chat_id: None,
                };
            }
        },
    };
    if checkpoint.status != SessionStatus::Archived {
        return RestoreResult {
            restored: false,
            notification_chat_id: None,
        };
    }
    // Compute notification chat_id for caller to send via outbound chain.
    let notification_chat_id = Some(
        checkpoint
            .peer_id
            .as_deref()
            .unwrap_or(session_id)
            .to_string(),
    );
    if let Err(e) = storage.restore_checkpoint(session_id).await {
        warn!(session_id = %session_id, error = %e,
            "failed to restore archived session");
        return RestoreResult {
            restored: false,
            notification_chat_id: None,
        };
    }
    RestoreResult {
        restored: true,
        notification_chat_id,
    }
}
