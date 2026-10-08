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
}
