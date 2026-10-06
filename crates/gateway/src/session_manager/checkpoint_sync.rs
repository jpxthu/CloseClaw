//! Checkpoint → live-session plan file path propagation.

use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::SessionCheckpoint;

/// Sync `plan_file_path` from checkpoint into ConversationSession
/// so Auto Mode plan injection survives process restarts.
pub(super) fn sync_plan_file_path_from_checkpoint(
    conv: &mut ConversationSession,
    cp: &SessionCheckpoint,
) {
    if let Some(ref ps) = cp.plan_state {
        if !ps.plan_file_path.is_empty() {
            conv.set_plan_file_path(Some(ps.plan_file_path.clone()));
        }
    }
}
