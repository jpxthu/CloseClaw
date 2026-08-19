//! Spawn tree reclaim GC — background sweep for residual nodes.
//!
//! Implements the "回收守护" (GC fallback) described in the design doc
//! (`docs/design/agent/agent-spawn.md` §回收守护). Scans the spawn tree
//! and reclaims two categories of residual nodes:
//!
//! 1. **Terminal滞留**: nodes with terminal status (Completed/Terminated)
//!    under an active parent that were not reclaimed by the normal
//!    announce path (e.g. push failure path).
//!
//! 2. **Orphaned parent**: parent session no longer exists in the
//!    `sessions` table (ended/archived) but child nodes remain in the
//!    tree. All children under such a parent are reclaimed.
//!
//! Called periodically from a background task that reuses the same
//! interval as the `ArchiveSweeper`.

use super::SessionManager;
use std::collections::HashSet;
use tracing::warn;

/// Sweep the spawn tree and reclaim residual nodes.
///
/// Scans every parent entry in `children.iter()` and performs two
/// categories of cleanup:
///
/// 1. For each parent that still exists in `sessions`, calls
///    `reclaim_completed()` to remove terminal-status children (滞留
///    「完成待回收」).
///
/// 2. For parents no longer in `sessions` (ended/archived), removes
///    all descendant entries — both terminal and active children are
///    cleaned up since the parent is gone.
///
/// Any reclaim action is logged at `warn` level since residual nodes
/// represent an abnormal path.
pub(crate) async fn sweep_spawn_tree_reclaim(session_manager: &SessionManager) {
    // Collect all parent IDs and their children snapshot.
    let snapshot: Vec<(String, Vec<closeclaw_session::spawn::ChildSessionInfo>)> = {
        let tree = session_manager.children.read().await;
        tree.iter()
            .map(|(parent_id, children)| (parent_id.clone(), children.clone()))
            .collect()
    };

    if snapshot.is_empty() {
        return;
    }

    // Step 1.9: Briefly hold sessions.read() to snapshot which parents are
    // still alive, then release the lock before processing children.write().
    // This avoids holding sessions.read() for the entire sweep duration.
    let active_parents: HashSet<String> = {
        let sessions = session_manager.sessions.read().await;
        snapshot
            .iter()
            .filter(|(parent_id, _)| sessions.contains_key(parent_id))
            .map(|(parent_id, _)| parent_id.clone())
            .collect()
    };

    // NOTE (Step 1.10): TOCTOU race window — `active_parents` snapshot may
    // become stale before we process each entry. A parent could be removed
    // from `sessions` between the snapshot and the per-entry processing,
    // causing an active parent to be treated as orphaned (condition ②).
    // This is low-risk: active children under such a parent are managed by
    // lifecycle linkage and will be cleaned up by the next GC sweep cycle.

    let mut reclaim_count: usize = 0;

    for (parent_id, children) in &snapshot {
        if active_parents.contains(parent_id) {
            reclaim_count += reclaim_terminal滞留(session_manager, parent_id).await;
        } else {
            reclaim_count +=
                remove_orphaned_descendants(session_manager, parent_id, children).await;
        }
    }

    // Log final summary if any reclaim occurred.
    if reclaim_count > 0 {
        warn!(
            total_reclaimed = reclaim_count,
            "spawn_reclaim_gc: sweep completed with reclaim actions"
        );
    }
}

/// Condition ①: Reclaim terminal滞留 nodes under an active parent.
///
/// Calls `reclaim_completed()` to remove children whose status is
/// Completed/Terminated. Returns the number of reclaimed nodes.
async fn reclaim_terminal滞留(session_manager: &SessionManager, parent_id: &str) -> usize {
    let mut tree = session_manager.children.write().await;
    let reclaimed = tree.reclaim_completed(parent_id);
    let count = reclaimed.len();
    if count > 0 {
        warn!(
            parent_id = %parent_id,
            reclaimed_count = count,
            reclaimed_ids = ?reclaimed,
            "spawn_reclaim_gc: reclaimed terminal滞留 nodes under active parent"
        );
    }
    count
}

/// Condition ②: Remove all descendants of an orphaned parent.
///
/// The parent session is no longer in the `sessions` table
/// (ended/archived). All children — both terminal and active — are
/// cleaned up. Returns the number of removed nodes.
async fn remove_orphaned_descendants(
    session_manager: &SessionManager,
    parent_id: &str,
    children: &[closeclaw_session::spawn::ChildSessionInfo],
) -> usize {
    let descendant_ids: Vec<String> = children
        .iter()
        .map(|info| info.session_id.clone())
        .collect();
    if descendant_ids.is_empty() {
        return 0;
    }
    let mut tree = session_manager.children.write().await;
    tree.remove_descendant_entries(&descendant_ids);
    warn!(
        parent_id = %parent_id,
        orphaned_count = descendant_ids.len(),
        orphaned_ids = ?descendant_ids,
        "spawn_reclaim_gc: reclaimed orphaned children (parent gone)"
    );
    descendant_ids.len()
}
