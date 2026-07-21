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
use closeclaw_config::agents::{AgentPermissionProvider, AgentPermissions};
use closeclaw_config::ConfigManager;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_helpers::{
    collect_chain_deny_subjects, collect_chain_effective_permissions,
};
use closeclaw_permission::engine::engine_types::Subject;
use closeclaw_session::spawn::context::SpawnCreationContext;
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

// ── SpawnCreationContext impl ────────────────────────────────────────────

#[async_trait::async_trait]
impl SpawnCreationContext for SessionManager {
    async fn get_parent_conversation_session(
        &self,
        parent_session_id: &str,
    ) -> Option<Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>> {
        self.get_conversation_session(parent_session_id).await
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Option<closeclaw_session::persistence::SessionCheckpoint> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        cm.load(session_id).await.ok().flatten()
    }

    async fn save_checkpoint(&self, cp: &closeclaw_session::persistence::SessionCheckpoint) {
        let cm = self.checkpoint_manager.read().await;
        if let Some(cm) = cm.as_ref() {
            if let Err(e) = cm.save_raw(cp).await {
                tracing::warn!(
                    session_id = %cp.session_id,
                    error = %e,
                    "failed to save child session checkpoint"
                );
            }
        }
    }

    fn get_agent_config(
        &self,
        agent_id: &str,
    ) -> Option<closeclaw_config::agents::ResolvedAgentConfig> {
        // Synchronous lookup from the config manager's in-memory agents map.
        let guard = self.config_manager.try_read().ok()?;
        let cm = (*guard).as_ref()?;
        let agents = cm.agents();
        agents.get(agent_id).cloned()
    }

    fn shutdown_signal(&self) -> Option<Arc<dyn closeclaw_common::ShutdownSignal>> {
        // Return the shutdown handle as a ShutdownSignal trait object.
        // Use try_read to avoid blocking; if unavailable, return None.
        let guard = self.shutdown_handle.try_read().ok()?;
        let handle = (*guard).clone()?;
        Some(handle)
    }

    fn default_reasoning_level(&self) -> closeclaw_session::persistence::ReasoningLevel {
        self.default_reasoning_level
    }

    fn llm_caller(&self) -> Option<Arc<dyn closeclaw_common::LlmCaller>> {
        let guard = self.llm_caller.try_read().ok()?;
        guard.clone()
    }

    fn system_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::SystemPromptBuilder>> {
        let guard = self.system_prompt_builder.try_read().ok()?;
        guard.clone()
    }

    fn prompt_overrides(&self) -> Option<closeclaw_common::PromptOverrides> {
        let guard = self.prompt_overrides.try_read().ok()?;
        guard.clone()
    }

    fn dynamic_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::DynamicPromptBuilder>> {
        let guard = self.dynamic_prompt_builder.try_read().ok()?;
        guard.clone()
    }

    fn skill_listing_provider(&self) -> Option<Arc<dyn closeclaw_common::SkillListingProvider>> {
        let guard = self.skill_listing_provider.try_read().ok()?;
        guard.clone()
    }

    async fn sender_id(&self, session_id: &str) -> Option<String> {
        self.get_sender_id(session_id).await
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
        // Resolve child permissions — early return if none configured.
        let child_perms = match self.config_manager.agent_permissions().get(child_agent_id) {
            Some(p) => p,
            None => return Ok(()),
        };

        // Resolve parent agent and effective permissions.
        let parent_agent_id = match self.session_manager.get_chat_id(parent_session_id).await {
            Some(id) => id,
            None => return Ok(()),
        };
        let agent_perms = self.config_manager.agent_permissions();
        let parent_perms = match collect_chain_effective_permissions(
            &*self.session_manager,
            agent_perms.as_ref(),
            parent_session_id,
            &parent_agent_id,
        )
        .await
        {
            Some(p) => p,
            None => return Ok(()),
        };

        // Evaluate user dimension and collect chain denials.
        let user_id = self.session_manager.get_sender_id(parent_session_id).await;
        let user_perms = self
            .evaluate_user_permissions(&user_id, child_agent_id)
            .await;
        let extra_deny = self
            .collect_chain_denials(parent_session_id, child_agent_id)
            .await;
        let extra_deny_ref = if extra_deny.is_empty() {
            None
        } else {
            Some(extra_deny.as_slice())
        };

        // Final intersection check.
        self.perform_inject_spawn_check(
            child_agent_id,
            &child_perms,
            &parent_perms,
            user_perms.as_ref(),
            user_id.as_deref(),
            extra_deny_ref,
        )
        .await
    }
}

// ── Permission evaluation helpers ─────────────────────────────────────

impl GatewayPermissionChecker {
    /// Perform the final permission intersection check via the engine.
    async fn perform_inject_spawn_check(
        &self,
        child_agent_id: &str,
        child_perms: &AgentPermissions,
        parent_perms: &AgentPermissions,
        user_perms: Option<&AgentPermissions>,
        user_id: Option<&str>,
        extra_deny: Option<&[Subject]>,
    ) -> Result<(), SpawnPermissionError> {
        self.permission_engine
            .read()
            .await
            .validate_and_inject_spawn(
                child_agent_id,
                child_perms,
                parent_perms,
                user_perms,
                user_id,
                extra_deny,
            )
            .map_err(|e| {
                tracing::debug!(error = %e, "spawn permission denied");
                SpawnPermissionError::Denied {
                    agent_id: child_agent_id.to_string(),
                    reason: e.to_string(),
                }
            })
    }

    /// Evaluate user-level permissions for the child agent.
    ///
    /// Owner (user_id = "owner") skips the User dimension per design doc:
    /// "Owner(User ID = 'owner') → skip User dim, only Agent".
    async fn evaluate_user_permissions(
        &self,
        user_id: &Option<String>,
        child_agent_id: &str,
    ) -> Option<AgentPermissions> {
        if user_id.as_deref() == Some("owner") {
            return None;
        }
        let uid = user_id.as_ref()?;
        Some(
            self.permission_engine
                .read()
                .await
                .evaluate_user_permissions(uid, child_agent_id),
        )
    }

    /// Collect deny subjects from the ancestor chain.
    async fn collect_chain_denials(
        &self,
        parent_session_id: &str,
        child_agent_id: &str,
    ) -> Vec<Subject> {
        let rules = self.permission_engine.read().await.rules().clone();
        collect_chain_deny_subjects(
            &*self.session_manager,
            &rules,
            parent_session_id,
            child_agent_id,
        )
        .await
    }
}
