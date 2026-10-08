//! Adapter: wraps the config crate's `LazyAgentPermissions` in
//! permission's [`AgentPermissionProvider`] port.
//!
//! `closeclaw-permission` does not depend on `closeclaw-config`; the
//! composition layer (daemon) adapts the config-side lazy permission
//! loader to the permission-domain port at assembly time, converting
//! between config-owned and permission-owned types field by field
//! (`agent_id` / `permissions` (eight-dimension `allowed` + limits'
//! `commands` / `paths` / `timeout_ms`) / `inherited_from`).

use std::sync::Arc;

use closeclaw_config::agents::{
    AgentPermissionProvider as ConfigAgentPermissionProvider,
    AgentPermissions as ConfigAgentPermissions, LazyAgentPermissions,
};
use closeclaw_permission::{
    ActionPermission, AgentPermissionProvider, AgentPermissions, PermissionLimits,
};

/// Map a config-side [`ConfigAgentPermissions`] to the permission-domain
/// [`AgentPermissions`], field by field (`agent_id` / `permissions`
/// (`allowed` + limits' `commands` / `paths` / `timeout_ms`) /
/// `inherited_from`).
fn to_permission_agent_permissions(p: &ConfigAgentPermissions) -> AgentPermissions {
    AgentPermissions {
        agent_id: p.agent_id.clone(),
        permissions: p
            .permissions
            .iter()
            .map(|(dim, action)| {
                (
                    dim.clone(),
                    ActionPermission {
                        allowed: action.allowed,
                        limits: PermissionLimits {
                            commands: action.limits.commands.clone(),
                            paths: action.limits.paths.clone(),
                            timeout_ms: action.limits.timeout_ms,
                        },
                    },
                )
            })
            .collect(),
        inherited_from: p.inherited_from.clone(),
    }
}

/// Wraps the config-side lazy permission loader in permission's
/// [`AgentPermissionProvider`] port for the daemon composition root.
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
    use closeclaw_config::agents::{
        ActionPermission as ConfigActionPermission, PermissionLimits as ConfigPermissionLimits,
    };
    use closeclaw_permission::{
        Effect, PermissionEngine, PermissionRequest, PermissionRequestBody, PermissionResponse,
    };
    use std::collections::HashMap;

    /// Build a config-side permissions entry with fully populated limits.
    fn config_action(allowed: bool) -> ConfigActionPermission {
        ConfigActionPermission {
            allowed,
            limits: ConfigPermissionLimits {
                commands: vec!["ls".into(), "cat".into()],
                paths: vec!["/tmp".into()],
                timeout_ms: Some(5000),
            },
        }
    }

    #[test]
    fn mapping_preserves_all_fields_including_limits() {
        let mut config = ConfigAgentPermissions::default();
        config.agent_id = "agent-a".into();
        config
            .permissions
            .insert("exec".to_string(), config_action(true));
        config
            .permissions
            .insert("file_write".to_string(), config_action(false));
        config.inherited_from = Some("parent".into());

        let mapped = to_permission_agent_permissions(&config);

        assert_eq!(mapped.agent_id, "agent-a");
        assert_eq!(mapped.inherited_from.as_deref(), Some("parent"));
        let exec = mapped.permissions.get("exec").unwrap();
        assert!(exec.allowed);
        assert_eq!(
            exec.limits.commands,
            vec!["ls".to_string(), "cat".to_string()]
        );
        assert_eq!(exec.limits.paths, vec!["/tmp".to_string()]);
        assert_eq!(exec.limits.timeout_ms, Some(5000));
        let file_write = mapped.permissions.get("file_write").unwrap();
        assert!(!file_write.allowed);
        assert_eq!(
            file_write.limits.commands,
            vec!["ls".to_string(), "cat".to_string()]
        );
        assert_eq!(file_write.limits.paths, vec!["/tmp".to_string()]);
        assert_eq!(file_write.limits.timeout_ms, Some(5000));
    }

    #[test]
    fn mapping_from_empty_config_yields_defaults() {
        let mapped = to_permission_agent_permissions(&ConfigAgentPermissions::default());
        assert_eq!(mapped, AgentPermissions::default());
    }

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
