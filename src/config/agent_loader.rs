//! Agent configuration loading for ConfigManager.
//!
//! Handles loading the agent registration list (agents.json, JSONC format),
//! resolving two-level directory configs, and validating parent_id references.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use tracing::info;

use super::agents::{AgentDirectoryProvider, AgentsConfig, ResolvedAgentConfig};
use super::manager::{ConfigLoadError, ConfigManager};

/// Strip `//` line comments from JSONC content.
fn strip_jsonc_comments(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            if let Some(idx) = line.find("//") {
                &line[..idx]
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl ConfigManager {
    /// Load agent registration list and resolve agent configurations
    /// from two-level directories (user + project).
    pub fn load_agents(&self, repo_root: Option<&Path>) -> Result<(), ConfigLoadError> {
        let agents_json_path = self.config_dir.join("agents.json");
        if !agents_json_path.exists() {
            return Ok(());
        }

        let raw = fs::read_to_string(&agents_json_path).map_err(|e| ConfigLoadError::IoError {
            path: agents_json_path.clone(),
            error: e.to_string(),
        })?;
        let cleaned = strip_jsonc_comments(&raw);
        let agents_cfg: AgentsConfig =
            serde_json::from_str(&cleaned).map_err(|e| ConfigLoadError::ParseError {
                path: agents_json_path.clone(),
                error: e.to_string(),
            })?;

        let user_agents_dir = self
            .config_dir
            .parent()
            .unwrap_or(&self.config_dir)
            .join("agents");
        let project_agents_dir = repo_root.map(|r| r.join(".closeclaw").join("agents"));

        let provider = AgentDirectoryProvider::new(
            agents_cfg.agents.clone(),
            user_agents_dir,
            project_agents_dir,
        )
        .map_err(|e| ConfigLoadError::ValidationError {
            path: agents_json_path.clone(),
            message: e.to_string(),
        })?;

        // Validate parent_id references against registry
        let registry_ids: HashSet<&str> = agents_cfg.agents.iter().map(String::as_str).collect();
        for (id, entry) in provider.entries() {
            if let Some(ref parent_id) = entry.parent_id {
                if !registry_ids.contains(parent_id.as_str()) {
                    return Err(ConfigLoadError::ValidationError {
                        path: agents_json_path,
                        message: format!(
                            "Agent '{}' references unregistered parent '{}'",
                            id, parent_id
                        ),
                    });
                }
            }
        }

        *self.agents.write().expect("RwLock for agents was poisoned") = provider.entries().clone();

        // Sync agent permissions
        let mut perms_map = self.agent_permissions.write().expect("RwLock poisoned");
        let provider_perms = provider.permissions();
        for id in &agents_cfg.agents {
            if let Some(p) = provider_perms.get(id) {
                perms_map.insert(id.clone(), p.clone());
            } else {
                // Registered agent missing permissions.json → synthesize full-deny baseline
                perms_map.insert(
                    id.clone(),
                    crate::agent::config::AgentPermissions {
                        agent_id: id.clone(),
                        permissions: std::collections::HashMap::new(),
                        inherited_from: None,
                    },
                );
            }
        }
        drop(perms_map);

        Ok(())
    }

    /// Hot-reload: re-scan agents/ directory and update agents + agent_permissions caches.
    /// Called when agents.json or any agents/<id>/*.json file changes on disk.
    pub fn reload_agents(&self) -> Result<(), ConfigLoadError> {
        self.load_agents(None)?;
        info!("Agent configs reloaded");
        Ok(())
    }

    /// Get all resolved agent configurations.
    pub fn agents(&self) -> HashMap<String, ResolvedAgentConfig> {
        self.agents
            .read()
            .expect("RwLock for agents was poisoned")
            .clone()
    }

    /// Get a single resolved agent configuration by ID.
    pub fn agent(&self, id: &str) -> Option<ResolvedAgentConfig> {
        self.agents
            .read()
            .expect("RwLock for agents was poisoned")
            .get(id)
            .cloned()
    }

    /// Get all agent permissions (clone).
    pub fn agent_permissions(&self) -> HashMap<String, crate::agent::config::AgentPermissions> {
        self.agent_permissions
            .read()
            .expect("RwLock for agent_permissions was poisoned")
            .clone()
    }
}
