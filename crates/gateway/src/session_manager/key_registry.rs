//! Session key registry rebuild logic.
//!
//! `rebuild_key_registry()` reconstructs the in-memory `key_registry`
//! (session_key → session_id) from persisted checkpoints at startup.

use super::SessionManager;
use closeclaw_session::persistence::PersistenceError;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tracing::warn;

impl SessionManager {
    /// Rebuild the `key_registry` (session_key → session_id) from persisted
    /// checkpoints.
    ///
    /// This is a **best-effort** reconstruction. Checkpoints created after
    /// the `sender_id` field was added carry the original `message.from`,
    /// allowing an exact match with `compute_session_key(PerChannelPeer)`
    /// (format: `{channel}:{from}:{to}`). For older checkpoints without
    /// `sender_id`, we fall back to using `agent_id`.
    ///
    /// When multiple sessions share the same reconstructed key, the one with
    /// the latest `created_at` is kept.
    pub async fn rebuild_key_registry(&self) -> Result<(), PersistenceError> {
        let cm_arc = {
            let guard = self.checkpoint_manager.read().await;
            match guard.as_ref() {
                Some(cm) => std::sync::Arc::clone(cm),
                None => {
                    // No storage configured — nothing to rebuild.
                    return Ok(());
                }
            }
        };

        // Collect session_ids from active sessions only.
        // Archived sessions are not loaded into the registry; they will be
        // restored on-demand via the SQLite fallback path in `resolve()`.
        let all_session_ids = cm_arc.storage().list_active_sessions().await?;

        // Accumulate: reconstructed key → (created_at, session_id)
        // Keep only the latest created_at per key.
        let mut key_best: HashMap<String, (chrono::DateTime<chrono::Utc>, String)> = HashMap::new();

        for session_id in &all_session_ids {
            let cp = match cm_arc.load(session_id).await {
                Ok(Some(cp)) => cp,
                Ok(None) => {
                    warn!(
                        session_id = %session_id,
                        "checkpoint returned None during rebuild, skipping"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(
                        session_id = %session_id,
                        error = %e,
                        "failed to load checkpoint during rebuild, skipping"
                    );
                    continue;
                }
            };

            // Extract routing fields.
            let platform = match cp.platform.as_deref() {
                Some(p) => p,
                None => {
                    // No platform in checkpoint — can't reconstruct key.
                    continue;
                }
            };
            let peer_id = match cp.peer_id.as_deref() {
                Some(id) => id,
                None => {
                    // No peer_id in checkpoint — can't reconstruct key.
                    continue;
                }
            };

            // Reconstruct the routing fields matching `compute_session_key(PerAccountChannelPeer)`
            // format: "{account_id}:{platform}:{sender_id}:{peer_id}".
            // Hash them to produce the same registry key as `resolve`.
            let account_id = cp.account_id.as_deref().unwrap_or("default");
            let from = cp
                .sender_id
                .as_deref()
                .unwrap_or_else(|| cp.agent_id.as_deref().unwrap_or(peer_id));
            let routing_fields = format!("{}:{}:{}:{}", account_id, platform, from, peer_id);
            let hash = Sha256::digest(routing_fields.as_bytes());
            let session_key = format!("{:x}", hash);

            let created = cp.created_at;
            match key_best.get(&session_key) {
                Some((existing_created, _)) if created <= *existing_created => {
                    // Existing entry is newer or equal — keep it.
                }
                _ => {
                    key_best.insert(session_key, (created, session_id.clone()));
                }
            }
        }

        // Write results into key_registry.
        {
            let mut registry = self.key_registry.write().await;
            for (key, (_, session_id)) in key_best {
                registry.insert(key, session_id);
            }
        }

        Ok(())
    }
}
