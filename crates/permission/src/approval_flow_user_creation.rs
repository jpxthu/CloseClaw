//! User creation approval and persistence helpers for [`ApprovalFlow`].

use super::ApprovalFlow;
use crate::user_registry::UserRegistry;
use closeclaw_common::permission_op::UserCreationRequest;

impl ApprovalFlow {
    /// Handle a user creation approval: register the user and persist rules.
    ///
    /// Returns `true` if the user was successfully registered.
    pub(super) async fn approve_user_creation(&mut self, request: &UserCreationRequest) -> bool {
        let user_id = &request.user_id;
        let channel = &request.im_channel;
        let initial_perms = &request.initial_permissions;

        // Load or create the in-memory registry (async, non-blocking).
        let registry_path = self.config_dir.join("users.json");
        let mut registry = {
            let path = registry_path.clone();
            let handle = self.runtime_handle.clone();
            let read_result = handle
                .spawn_blocking(move || {
                    if path.exists() {
                        std::fs::read_to_string(&path)
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "registry file does not exist",
                        ))
                    }
                })
                .await;
            match read_result {
                Ok(Ok(data)) => serde_json::from_str::<UserRegistry>(&data).unwrap_or_default(),
                _ => UserRegistry::new(),
            }
        };

        // Register user and generate permission rules.
        let ruleset = match registry.register_user(user_id, channel, initial_perms) {
            Ok(rs) => rs,
            Err(crate::user_registry::RegistryError::AlreadyRegistered(_)) => {
                tracing::warn!(
                    user_id = %user_id,
                    "user already registered, skipping"
                );
                return false;
            }
        };

        // Persist user registry.
        self.persist_user_registry(&registry_path, &registry);

        // Persist initial permission rules to agent's permissions.json.
        self.persist_initial_permission_rules(user_id, &ruleset);

        // Trigger permission engine hot-reload.
        (self.on_whitelist_updated)(user_id);

        true
    }

    /// Persist the user registry to disk (async, non-blocking).
    pub(super) fn persist_user_registry(
        &self,
        registry_path: &std::path::Path,
        registry: &UserRegistry,
    ) {
        let registry_path = registry_path.to_path_buf();
        let json = match serde_json::to_string_pretty(registry) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize user registry");
                return;
            }
        };
        let handle = self.runtime_handle.clone();
        handle.spawn_blocking(move || {
            if let Some(parent) = registry_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    tracing::warn!(
                        path = %parent.display(),
                        error = %e,
                        "failed to create registry directory"
                    );
                    return;
                }
            }
            if let Err(e) = std::fs::write(&registry_path, json) {
                tracing::warn!(
                    path = %registry_path.display(),
                    error = %e,
                    "failed to write user registry"
                );
            }
        });
    }

    /// Persist initial permission rules to the agent's permissions.json
    /// (async, non-blocking).
    pub(super) fn persist_initial_permission_rules(
        &self,
        user_id: &str,
        new_rules: &crate::engine::engine_types::RuleSet,
    ) {
        let path = self
            .config_dir
            .join("agents")
            .join(user_id)
            .join("permissions.json");
        let new_rules = new_rules.clone();
        let handle = self.runtime_handle.clone();
        handle.spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    tracing::warn!(
                        path = %parent.display(),
                        error = %e,
                        "failed to create agent permissions directory"
                    );
                    return;
                }
            }

            // Read existing rules or start fresh.
            let mut ruleset: crate::engine::engine_types::RuleSet = if path.exists() {
                std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|data| serde_json::from_str(&data).ok())
                    .unwrap_or_default()
            } else {
                crate::engine::engine_types::RuleSet::default()
            };

            // Append new rules.
            ruleset.rules.extend(new_rules.rules);

            // Write back.
            match serde_json::to_string_pretty(&ruleset) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&path, json) {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "failed to write permissions.json for user"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to serialize permissions.json"
                    );
                }
            }
        });
    }
}
