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
pub async fn sweep_spawn_tree_reclaim(session_manager: &SessionManager) {
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

    let mut reclaim_count: usize = 0;

    // Check which parent sessions are still active.
    let sessions = session_manager.sessions.read().await;

    for (parent_id, children) in &snapshot {
        if sessions.contains_key(parent_id) {
            // Condition ①: Parent active — reclaim terminal滞留 nodes.
            let mut tree = session_manager.children.write().await;
            let reclaimed = tree.reclaim_completed(parent_id);
            let count = reclaimed.len();
            if count > 0 {
                reclaim_count += count;
                warn!(
                    parent_id = %parent_id,
                    reclaimed_count = count,
                    reclaimed_ids = ?reclaimed,
                    "spawn_reclaim_gc: reclaimed terminal滞留 nodes under active parent"
                );
            }
        } else {
            // Condition ②: Parent gone — remove all child entries.
            let descendant_ids: Vec<String> = children
                .iter()
                .map(|info| info.session_id.clone())
                .collect();
            if !descendant_ids.is_empty() {
                let mut tree = session_manager.children.write().await;
                tree.remove_descendant_entries(&descendant_ids);
                reclaim_count += descendant_ids.len();
                warn!(
                    parent_id = %parent_id,
                    orphaned_count = descendant_ids.len(),
                    orphaned_ids = ?descendant_ids,
                    "spawn_reclaim_gc: reclaimed orphaned children (parent gone)"
                );
            }
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
