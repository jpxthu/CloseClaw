//! Permission Engine - Chain-aware evaluation for general operations.
//!
//! Adds dimension-level intersection with the parent agent chain,
//! closing the gap between spawn-time validation and general-operation
//! permission checks.

use super::engine_eval::PermissionEngine;
use super::engine_helpers::{collect_chain_deny_subjects, collect_chain_effective_permissions};
use super::engine_risk::assess_risk_level;
use super::engine_types::{PermissionRequest, PermissionResponse};
use closeclaw_common::SessionLookup;
use closeclaw_config::agents::{AgentPermissionProvider, AgentPermissions};
use tracing::info;

impl PermissionEngine {
    /// Evaluate a permission request with parent-agent chain intersection.
    ///
    /// This method extends [`Self::evaluate`] by also checking the
    /// dimension-level intersection with the parent agent chain.  It
    /// is the general-operation counterpart of the spawn-time
    /// `validate_and_inject_spawn` check.
    ///
    /// # Implementation
    ///
    /// 1. Collect chain effective permissions via
    ///    [`collect_chain_effective_permissions`].
    /// 2. Collect chain deny subjects via
    ///    [`collect_chain_deny_subjects`].
    /// 3. Call [`Self::evaluate`] with the deny subjects.
    /// 4. If chain effective permissions exist, check whether the
    ///    requested dimension is denied in the chain intersection.
    ///    A denied dimension overrides the evaluate result.
    ///
    /// # Design constraints
    ///
    /// - **Child-agent self-Deny**: handled by `evaluate()` rule
    ///   matching; if the child is denied by its own rules, evaluate
    ///   returns `Denied` before reaching the chain check.
    /// - **No caching**: each call computes permissions fresh.
    /// - **Workspace forced auth**: `evaluate()` handles this in
    ///   Step 0.5; the chain check is applied after evaluate returns,
    ///   so workspace authorization is not disrupted.
    /// - **Sub-agent Deny → no user approval**: when this method
    ///   returns `Denied`, the caller should surface an error directly
    ///   rather than entering the user-approval flow.
    pub async fn evaluate_with_chain(
        &self,
        request: PermissionRequest,
        session_manager: &dyn SessionLookup,
        session_id: &str,
        agent_permissions: &dyn AgentPermissionProvider,
    ) -> PermissionResponse {
        let agent_id = request.agent_id().to_string();

        // --- Step 1: Collect chain effective permissions ---
        let chain_effective = self
            .collect_chain_effective(session_manager, session_id, agent_permissions)
            .await;

        // --- Step 2: Collect chain deny subjects ---
        let deny_subjects =
            collect_chain_deny_subjects(session_manager, &self.rules, session_id, &agent_id).await;
        let extra_deny = if deny_subjects.is_empty() {
            None
        } else {
            Some(deny_subjects)
        };

        // --- Step 3: Evaluate with deny subjects ---
        let response = self.evaluate(request.clone(), extra_deny);

        // --- Step 4: Dimension-level chain intersection check ---
        if let Some(ref chain_perms) = chain_effective {
            if let Some(dim) = request.body().dimension_name() {
                if let Some(dim_perm) = chain_perms.permissions.get(dim) {
                    if !dim_perm.allowed {
                        info!(
                            agent = %agent_id,
                            dimension = %dim,
                            result = "denied",
                            reason = "chain_dimension_deny",
                            "permission check completed"
                        );
                        return PermissionResponse::Denied {
                            reason: format!(
                                "action denied by parent agent chain: \
                                 dimension '{}' denied by ancestor",
                                dim
                            ),
                            rule: "<chain_intersection>".to_string(),
                            risk_level: assess_risk_level(request.body()),
                        };
                    }
                }
            }
        }

        response
    }

    /// Walk the parent chain from `session_id` upward and compute the
    /// intersection of all ancestors' configured permissions.
    ///
    /// Returns `None` when the direct parent (or any ancestor in the
    /// chain) has no configured permissions, matching the behavior of
    /// [`collect_chain_effective_permissions`].
    async fn collect_chain_effective(
        &self,
        session_manager: &dyn SessionLookup,
        session_id: &str,
        agent_permissions: &dyn AgentPermissionProvider,
    ) -> Option<AgentPermissions> {
        let parent_session_id = session_manager.get_parent_of(session_id).await?;
        let parent_agent_id = session_manager.get_chat_id(&parent_session_id).await?;
        collect_chain_effective_permissions(
            session_manager,
            agent_permissions,
            &parent_session_id,
            &parent_agent_id,
        )
        .await
    }
}
