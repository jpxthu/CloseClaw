//! Agent configuration loading for ConfigManager.
//!
//! Handles loading the agent registration list (agents.json, JSONC format),
//! resolving two-level directory configs, and validating parent_id references.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use tracing::info;

use crate::agents::{
    strip_jsonc_comments, AgentDirectoryProvider, AgentsConfig, ResolvedAgentConfig,
};
use crate::manager::{ConfigLoadError, ConfigManager};

impl ConfigManager {
    /// Load an agents.json file and return parsed agent IDs.
    pub fn load_agents_json(&self, path: &Path) -> Result<Vec<String>, ConfigLoadError> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(path).map_err(|e| ConfigLoadError::IoError {
            path: path.to_path_buf(),
            error: e.to_string(),
        })?;
        let cleaned = strip_jsonc_comments(&raw);
        let cfg: AgentsConfig =
            serde_json::from_str(&cleaned).map_err(|e| ConfigLoadError::ParseError {
                path: path.to_path_buf(),
                error: e.to_string(),
            })?;
        Ok(cfg.agents)
    }

    /// Merge agent IDs from user and project lists (union).
    pub fn merge_agent_ids(user: &[String], project: &[String]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut merged = Vec::new();
        for id in user.iter().chain(project.iter()) {
            if seen.insert(id.clone()) {
                merged.push(id.clone());
            }
        }
        merged
    }

    /// Validate parent_id references against the merged registry.
    fn validate_parent_ids(
        provider: &AgentDirectoryProvider,
        registry_ids: &HashSet<&str>,
        json_path: &Path,
    ) -> Result<(), ConfigLoadError> {
        for (id, entry) in provider.entries() {
            if let Some(ref parent_id) = entry.parent_id {
                if !registry_ids.contains(parent_id.as_str()) {
                    return Err(ConfigLoadError::ValidationError {
                        path: json_path.to_path_buf(),
                        message: format!(
                            "Agent '{}' references unregistered parent '{}'",
                            id, parent_id
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Sync agent permissions from provider into ConfigManager cache.
    fn sync_permissions(
        &self,
        ids: &[String],
        provider_perms: &HashMap<String, crate::agents::AgentPermissions>,
    ) {
        let mut perms_map = self.agent_permissions.write().expect("RwLock poisoned");
        for id in ids {
            if let Some(p) = provider_perms.get(id) {
                perms_map.insert(id.clone(), p.clone());
            } else {
                perms_map.insert(
                    id.clone(),
                    crate::agents::AgentPermissions {
                        agent_id: id.clone(),
                        permissions: std::collections::HashMap::new(),
                        inherited_from: None,
                    },
                );
            }
        }
    }

    /// Load agent registration list and resolve agent configurations
    /// from two-level directories (user + project).
    pub fn load_agents(&self, repo_root: Option<&Path>) -> Result<(), ConfigLoadError> {
        // Persist repo_root so reload_agents() can load project-level config
        if repo_root.is_some() {
            *self.repo_root.write().expect("RwLock poisoned") = repo_root.map(Path::to_path_buf);
        }

        let agents_json_path = self.config_dir.join("agents.json");
        let user_ids = self.load_agents_json(&agents_json_path)?;

        let project_ids = if let Some(repo) = repo_root {
            let path = repo.join(".closeclaw").join("agents.json");
            self.load_agents_json(&path)?
        } else {
            Vec::new()
        };

        let merged_ids = Self::merge_agent_ids(&user_ids, &project_ids);
        if merged_ids.is_empty() {
            return Ok(());
        }

        let user_agents_dir = self
            .config_dir
            .parent()
            .unwrap_or(&self.config_dir)
            .join("agents");
        let project_agents_dir = repo_root.map(|r| r.join(".closeclaw").join("agents"));

        let provider = {
            // Extract global memory config from the loaded sections.
            let global_memory = self
                .sections
                .read()
                .expect("RwLock for config sections was poisoned")
                .get(&crate::manager::ConfigSection::Memory)
                .and_then(|v| {
                    serde_json::from_value::<crate::providers::memory::MemoryConfigData>(v.clone())
                        .ok()
                })
                .map(|d| d.config);
            AgentDirectoryProvider::new(
                merged_ids.clone(),
                user_agents_dir,
                project_agents_dir,
                global_memory,
            )
        }
        .map_err(|e| ConfigLoadError::ValidationError {
            path: agents_json_path.clone(),
            message: e.to_string(),
        })?;

        let registry_ids: HashSet<&str> = merged_ids.iter().map(String::as_str).collect();
        Self::validate_parent_ids(&provider, &registry_ids, &agents_json_path)?;

        *self.agents.write().expect("RwLock for agents was poisoned") = provider.entries().clone();
        self.sync_permissions(&merged_ids, provider.permissions());

        Ok(())
    }

    /// Hot-reload: re-scan agents/ directory and update caches.
    pub fn reload_agents(&self) -> Result<(), ConfigLoadError> {
        let repo_root = self.repo_root.read().expect("RwLock poisoned").clone();
        self.load_agents(repo_root.as_deref())?;
        info!("Agent configs reloaded");
        Ok(())
    }

    /// Restore agents and permissions from a snapshot.
    ///
    /// Used by the daemon to roll back agent state when `reload_agents()`
    /// fails. Call `snapshot_agents()` before reloading, then pass the
    /// snapshot here on failure.
    pub fn restore_agents(
        &self,
        agents: HashMap<String, ResolvedAgentConfig>,
        permissions: HashMap<String, crate::agents::AgentPermissions>,
    ) {
        *self.agents.write().expect("RwLock for agents was poisoned") = agents;
        *self
            .agent_permissions
            .write()
            .expect("RwLock for agent_permissions was poisoned") = permissions;
    }

    /// Snapshot the current agents and permissions for later rollback.
    pub fn snapshot_agents(
        &self,
    ) -> (
        HashMap<String, ResolvedAgentConfig>,
        HashMap<String, crate::agents::AgentPermissions>,
    ) {
        let agents = self.agents();
        let permissions = self.agent_permissions();
        (agents, permissions)
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
    pub fn agent_permissions(&self) -> HashMap<String, crate::agents::AgentPermissions> {
        self.agent_permissions
            .read()
            .expect("RwLock for agent_permissions was poisoned")
            .clone()
    }
}
