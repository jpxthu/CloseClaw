//! Composition-root shell: wraps the config crate's `LazyAgentPermissions`
//! in permission's [`AgentPermissionProvider`] port.
//!
//! `closeclaw-permission` does not depend on `closeclaw-config`; the
//! field-level config→permission mapping (`agent_id` / `permissions`
//! (eight-dimension `allowed` + limits' `commands` / `paths` /
//! `timeout_ms`) / `inherited_from`) lives in the gateway bridge module
//! (`closeclaw_gateway::agent_permissions_bridge`). This daemon adapter
//! only performs the assembly (wrap + delegate) at the composition root.

use std::sync::Arc;

use closeclaw_config::agents::{
    AgentPermissionProvider as ConfigAgentPermissionProvider, LazyAgentPermissions,
};
use closeclaw_gateway::agent_permissions_bridge::to_permission_agent_permissions;
use closeclaw_permission::{AgentPermissionProvider, AgentPermissions};

/// Wraps the config-side lazy permission loader in permission's
/// [`AgentPermissionProvider`] port for the daemon composition root.
/// Mapping delegates to the gateway shared bridge.
pub(crate) struct AgentPermissionsAdapter {
    inner: Arc<LazyAgentPermissions>,
}

impl AgentPermissionsAdapter {
    /// Wrap the given config-side lazy permission loader.
    pub(crate) fn new(inner: Arc<LazyAgentPermissions>) -> Self {
        Self { inner }
    }
}

impl AgentPermissionProvider for AgentPermissionsAdapter {
    fn get(&self, agent_id: &str) -> Option<AgentPermissions> {
        ConfigAgentPermissionProvider::get(self.inner.as_ref(), agent_id)
            .as_ref()
            .map(to_permission_agent_permissions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_permission::{
        Effect, PermissionEngine, PermissionRequest, PermissionRequestBody, PermissionResponse,
    };
    use std::collections::HashMap;

    #[test]
    fn get_returns_none_for_agent_without_permissions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let adapter = AgentPermissionsAdapter::new(Arc::new(LazyAgentPermissions::new(
            tmp.path().to_path_buf(),
        )));
        assert!(adapter.get("unknown-agent").is_none());
    }

    // ── End-to-end: adapter-injected evaluate_with_chain verdicts ────

    /// Minimal `SessionLookup` double registering a parent→child spawn
    /// chain (the permission crate's `MockSessionLookup` is `#[cfg(test)]`
    /// and not exported).
    struct ChainLookup {
        parents: HashMap<String, String>,
        agents: HashMap<String, String>,
    }

    impl ChainLookup {
        fn parent_child(
            parent_session: &str,
            parent_agent: &str,
            child_session: &str,
            child_agent: &str,
        ) -> Self {
            Self {
                parents: HashMap::from([(child_session.to_string(), parent_session.to_string())]),
                agents: HashMap::from([
                    (parent_session.to_string(), parent_agent.to_string()),
                    (child_session.to_string(), child_agent.to_string()),
                ]),
            }
        }
    }

    #[async_trait::async_trait]
    impl closeclaw_common::SessionLookup for ChainLookup {
        async fn get_parent_of(&self, child_id: &str) -> Option<String> {
            self.parents.get(child_id).cloned()
        }

        async fn get_chat_id(&self, session_id: &str) -> Option<String> {
            self.agents.get(session_id).cloned()
        }

        async fn push_pending_message(
            &self,
            _: &str,
            _: closeclaw_common::session_lookup::PendingMessage,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn get_plan_state(&self, _: &str) -> Option<closeclaw_common::PlanState> {
            None
        }

        async fn set_plan_state(&self, _: &str, _: closeclaw_common::PlanState) {}

        async fn set_session_mode(&self, _: &str, _: closeclaw_common::SessionMode) {}
    }

    /// Write a config-side `agents/parent/permissions.json` fixture:
    /// every dimension allowed except `file_write`.
    fn write_parent_permissions(config_dir: &std::path::Path) {
        let parent_dir = config_dir.join("agents").join("parent");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(
            parent_dir.join("permissions.json"),
            serde_json::json!({
                "agent_id": "parent",
                "permissions": {
                    "exec": { "allowed": true },
                    "file_read": { "allowed": true },
                    "file_write": { "allowed": false },
                    "network": { "allowed": true },
                    "spawn": { "allowed": true },
                    "tool_call": { "allowed": true },
                    "config_write": { "allowed": true },
                    "message": { "allowed": true },
                },
            })
            .to_string(),
        )
        .unwrap();
    }

    /// Engine with all-allow defaults (same shape as the permission
    /// crate's chain tests); agent-rule lazy loading disabled so no
    /// disk reads happen outside the config fixture.
    fn all_allow_engine() -> PermissionEngine {
        let ruleset = closeclaw_permission::RuleSetBuilder::new()
            .default_file_read(Effect::Allow)
            .default_file_write(Effect::Allow)
            .default_exec(Effect::Allow)
            .default_network(Effect::Allow)
            .default_inter_agent(Effect::Allow)
            .default_tool_call(Effect::Allow)
            .default_config(Effect::Allow)
            .build()
            .unwrap();
        PermissionEngine::new_with_default_data_root(ruleset)
    }

    /// Adapter injected into the production `evaluate_with_chain` path:
    /// the parent's config-side `permissions.json` (file_write denied)
    /// must deny the child's file write with the chain reason, while
    /// file read stays allowed — the same verdicts the engine produced
    /// with config-side types before the port migration.
    #[tokio::test]
    async fn injected_evaluate_with_chain_denies_parent_denied_dimension() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_parent_permissions(tmp.path());
        let adapter = AgentPermissionsAdapter::new(Arc::new(LazyAgentPermissions::new(
            tmp.path().to_path_buf(),
        )));
        let engine = all_allow_engine();
        let lookup =
            ChainLookup::parent_child("parent-session", "parent", "child-session", "child");

        let write = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "child".to_string(),
            path: "/tmp/test.txt".to_string(),
            op: "write".to_string(),
        });
        let resp = engine
            .evaluate_with_chain(write, &lookup, "child-session", &adapter)
            .await;
        assert!(
            matches!(resp, PermissionResponse::Denied { ref reason, .. } if reason.contains("chain")),
            "file_write should be denied by parent chain: {:?}",
            resp
        );

        let read = PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "child".to_string(),
            path: "/tmp/test.txt".to_string(),
            op: "read".to_string(),
        });
        let resp = engine
            .evaluate_with_chain(read, &lookup, "child-session", &adapter)
            .await;
        assert!(
            matches!(resp, PermissionResponse::Allowed { .. }),
            "file_read should stay allowed: {:?}",
            resp
        );
    }
}
