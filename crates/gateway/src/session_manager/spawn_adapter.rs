//! Gateway-side implementations of `SpawnContext` and `PermissionChecker` traits.
//!
//! `SpawnContext` is implemented on `SessionManager` to bridge the async
//! session queries (child counts, chat_id, sender_id, depth budget) to
//! the `SpawnController` in the session crate.
//!
//! `GatewayPermissionChecker` wraps `PermissionEngine` + `ConfigManager`
//! to implement the `PermissionChecker` trait, performing the full chain
//! evaluation, user dimension, and deny-subject checks.

use std::sync::Arc;

use closeclaw_common::{PermissionChecker, SpawnPermissionError};
use closeclaw_config::agents::AgentPermissionProvider;
use closeclaw_config::ConfigManager;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_helpers::{
    collect_chain_deny_subjects, collect_chain_effective_permissions,
};
use closeclaw_session::spawn::controller::SpawnContext;

use super::SessionManager;

// ── SpawnContext impl ───────────────────────────────────────────────────

#[async_trait::async_trait]
impl SpawnContext for SessionManager {
    async fn active_children_count(&self, parent_session_id: &str) -> usize {
        self.count_active_children(parent_session_id).await
    }

    async fn chat_id(&self, session_id: &str) -> Option<String> {
        self.get_chat_id(session_id).await
    }

    async fn sender_id(&self, session_id: &str) -> Option<String> {
        self.get_sender_id(session_id).await
    }

    async fn effective_max_spawn_depth(&self, session_id: &str) -> Option<u32> {
        self.get_effective_max_spawn_depth(session_id).await
    }
}

// ── GatewayPermissionChecker ────────────────────────────────────────────

/// Gateway-side implementation of [`PermissionChecker`].
///
/// Wraps `PermissionEngine` (for user-level permission evaluation and
/// spawn intersection) and `ConfigManager` (for agent-level permission
/// configs). Uses `SessionManager` (as `SessionLookup`) for chain
/// traversal.
pub struct GatewayPermissionChecker {
    session_manager: Arc<SessionManager>,
    config_manager: Arc<ConfigManager>,
    permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
}

impl GatewayPermissionChecker {
    pub fn new(
        session_manager: Arc<SessionManager>,
        config_manager: Arc<ConfigManager>,
        permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    ) -> Self {
        Self {
            session_manager,
            config_manager,
            permission_engine,
        }
    }
}

#[async_trait::async_trait]
impl PermissionChecker for GatewayPermissionChecker {
    async fn validate_spawn_permission(
        &self,
        child_agent_id: &str,
        parent_session_id: &str,
    ) -> Result<(), SpawnPermissionError> {
        let child_perms = self.config_manager.agent_permissions().get(child_agent_id);
        let agent_perms = self.config_manager.agent_permissions();
        let parent_agent_id = self.session_manager.get_chat_id(parent_session_id).await;

        let Some(child_perms) = child_perms else {
            // No permissions configured for child — no restriction.
            return Ok(());
        };

        let Some(parent_agent_id) = parent_agent_id else {
            // No parent agent — skip permission check.
            return Ok(());
        };

        let parent_perms = collect_chain_effective_permissions(
            &*self.session_manager,
            agent_perms.as_ref(),
            parent_session_id,
            &parent_agent_id,
        )
        .await;

        let Some(parent_perms) = parent_perms else {
            // Parent has no configured permissions — no restriction.
            return Ok(());
        };

        let user_id = self.session_manager.get_sender_id(parent_session_id).await;

        // Owner skips User dimension — design doc:
        // "Owner(User ID = 'owner') → skip User dim, only Agent"
        let user_perms = if user_id.as_deref() == Some("owner") {
            None
        } else if let Some(ref uid) = user_id {
            Some(
                self.permission_engine
                    .read()
                    .await
                    .evaluate_user_permissions(uid, child_agent_id),
            )
        } else {
            None
        };

        // Collect full-chain deny subjects from all ancestors.
        let rules = self.permission_engine.read().await.rules().clone();
        let chain_deny_subjects = collect_chain_deny_subjects(
            &*self.session_manager,
            &rules,
            parent_session_id,
            child_agent_id,
        )
        .await;
        let extra_deny = if chain_deny_subjects.is_empty() {
            None
        } else {
            Some(chain_deny_subjects.as_slice())
        };

        self.permission_engine
            .read()
            .await
            .validate_and_inject_spawn(
                child_agent_id,
                &child_perms,
                &parent_perms,
                user_perms.as_ref(),
                user_id.as_deref(),
                extra_deny,
            )
            .map_err(|e| {
                tracing::debug!(error = %e, "spawn permission denied");
                SpawnPermissionError::Denied {
                    agent_id: child_agent_id.to_string(),
                    reason: e.to_string(),
                }
            })?;

        Ok(())
    }
}
