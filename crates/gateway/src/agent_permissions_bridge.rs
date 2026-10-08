//! Shared config→permission boundary bridge.
//!
//! `closeclaw-permission` does not depend on `closeclaw-config`; the
//! gateway crate is the registered cross-domain bridge (wrapping the
//! permission engine and reading config bindings — see
//! `docs/design/STANDARDS.md` 依赖方向允许边表). This module adapts the
//! config-side lazy permission loader to the permission-domain
//! [`AgentPermissionProvider`] port, converting between config-owned
//! and permission-owned types field by field (`agent_id` /
//! `permissions` (eight-dimension `allowed` + limits' `commands` /
//! `paths` / `timeout_ms`) / `inherited_from`). The daemon composition
//! root and the tools test adapters reuse these shared items instead
//! of keeping local copies.

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
pub fn to_permission_agent_permissions(p: &ConfigAgentPermissions) -> AgentPermissions {
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

/// Adapter exposing the config-side lazy permission loader as the
/// permission-side [`AgentPermissionProvider`] port.
pub struct ConfigAgentPermissionsProviderAdapter {
    inner: Arc<LazyAgentPermissions>,
}

impl ConfigAgentPermissionsProviderAdapter {
    /// Wrap the given config-side lazy permission loader.
    pub fn new(inner: Arc<LazyAgentPermissions>) -> Self {
        Self { inner }
    }
}

impl AgentPermissionProvider for ConfigAgentPermissionsProviderAdapter {
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

    /// Write a config-side `agents/<id>/permissions.json` fixture: every
    /// dimension allowed except `file_write`.
    fn write_permissions(config_dir: &std::path::Path, agent_id: &str) {
        let agent_dir = config_dir.join("agents").join(agent_id);
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("permissions.json"),
            serde_json::json!({
                "agent_id": agent_id,
                "permissions": {
                    "exec": { "allowed": true },
                    "file_write": { "allowed": false },
                },
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn provider_get_maps_known_agent_and_returns_none_for_unknown() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_permissions(tmp.path(), "agent-a");
        let adapter = ConfigAgentPermissionsProviderAdapter::new(Arc::new(
            LazyAgentPermissions::new(tmp.path().to_path_buf()),
        ));

        let mapped = adapter.get("agent-a").expect("known agent");
        assert_eq!(mapped.agent_id, "agent-a");
        assert!(mapped.permissions.get("exec").unwrap().allowed);
        assert!(!mapped.permissions.get("file_write").unwrap().allowed);

        assert!(adapter.get("unknown-agent").is_none());
    }
}
