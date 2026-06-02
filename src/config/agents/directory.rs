//! Agent Directory Provider — loads per-agent config from two-level directories.
//!
//! Scans user-level and project-level agent directories, filters by
//! a registration list (from agents.json), and merges configs using
//! field-level override (project > user).

use std::collections::HashMap;
use std::path::PathBuf;

use tracing::warn;

use crate::agent::config::{AgentConfig, AgentPermissions};
use crate::config::agents::resolved::{ConfigSource, ResolvedAgentConfig};
use crate::config::ConfigError;

/// Loads agent configurations from user-level and optional project-level
/// directories, filtered by a registration list.
#[derive(Debug)]
pub struct AgentDirectoryProvider {
    registry: Vec<String>,
    user_agents_dir: PathBuf,
    project_agents_dir: Option<PathBuf>,
    entries: HashMap<String, ResolvedAgentConfig>,
    permissions: HashMap<String, AgentPermissions>,
}

impl AgentDirectoryProvider {
    /// Create a new provider.
    ///
    /// - `registry`: agent IDs from the registration list (agents.json)
    /// - `user_agents_dir`: e.g. `~/.closeclaw/agents/`
    /// - `project_agents_dir`: e.g. `<repo>/.closeclaw/agents/` (optional)
    pub fn new(
        registry: Vec<String>,
        user_agents_dir: PathBuf,
        project_agents_dir: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let mut provider = Self {
            registry,
            user_agents_dir,
            project_agents_dir,
            entries: HashMap::new(),
            permissions: HashMap::new(),
        };
        provider.reload()?;
        Ok(provider)
    }

    /// Reload all agent configs from disk.
    pub fn reload(&mut self) -> Result<(), ConfigError> {
        self.entries.clear();
        self.permissions.clear();

        for id in &self.registry {
            let user_config_path = self.user_agents_dir.join(id).join("config.json");
            let project_config_path = self
                .project_agents_dir
                .as_ref()
                .map(|d| d.join(id).join("config.json"));

            // Try loading configs from both levels
            let mut user_config = Self::load_agent_config(&user_config_path);
            let mut project_config = project_config_path
                .as_ref()
                .and_then(|p| Self::load_agent_config(p));

            // Backfill `id` from the directory name when missing, and
            // warn on mismatch. The project layer is processed first so
            // its injected id wins when both levels have an empty id
            // (project fields override user fields in the merge below).
            Self::inject_dirname_id(&mut project_config, id);
            Self::inject_dirname_id(&mut user_config, id);

            let path_label = id.as_str();
            let resolved = match (project_config, user_config) {
                (Some(proj), Some(usr)) => ResolvedAgentConfig::merge(proj, usr, path_label)?,
                (Some(proj), None) => {
                    ResolvedAgentConfig::from_single(proj, ConfigSource::Project, path_label)?
                }
                (None, Some(usr)) => {
                    ResolvedAgentConfig::from_single(usr, ConfigSource::User, path_label)?
                }
                (None, None) => {
                    warn!("Agent '{}' in registry but no config.json found", id);
                    continue;
                }
            };

            // Load permissions (same priority: project > user)
            let user_perm_path = self.user_agents_dir.join(id).join("permissions.json");
            let project_perm_path = self
                .project_agents_dir
                .as_ref()
                .map(|d| d.join(id).join("permissions.json"));

            let perms = project_perm_path
                .as_ref()
                .and_then(|p| Self::load_permissions(p))
                .or_else(|| Self::load_permissions(&user_perm_path));

            if let Some(p) = perms {
                self.permissions.insert(id.clone(), p);
            }

            self.entries.insert(id.clone(), resolved);
        }

        Ok(())
    }

    fn load_agent_config(path: &std::path::Path) -> Option<AgentConfig> {
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Backfill the agent's `id` from the directory name when missing,
    /// and warn when the file's `id` disagrees with the directory name.
    ///
    /// `AgentConfig.id` deserializes as an empty string by default, so a
    /// config.json that omits `id` is not an error here — the directory
    /// name from the registration list is the canonical id. A non-empty
    /// `id` that differs from the directory name is almost always a
    /// misconfiguration, so we log it at WARN level and keep the file's
    /// value (the downstream merge / `from_single` will use whatever
    /// `id` is on the config object).
    fn inject_dirname_id(config: &mut Option<AgentConfig>, dirname: &str) {
        if let Some(cfg) = config.as_mut() {
            if cfg.id.is_empty() {
                cfg.id = dirname.to_string();
            } else if cfg.id != dirname {
                warn!(
                    agent_id = %cfg.id,
                    dirname = %dirname,
                    "agent config id does not match directory name; using config id"
                );
            }
        }
    }

    fn load_permissions(path: &std::path::Path) -> Option<AgentPermissions> {
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Get a resolved agent config by ID.
    pub fn get(&self, id: &str) -> Option<&ResolvedAgentConfig> {
        self.entries.get(id)
    }

    /// Get all resolved entries.
    pub fn entries(&self) -> &HashMap<String, ResolvedAgentConfig> {
        &self.entries
    }

    /// Get all loaded permissions.
    pub fn permissions(&self) -> &HashMap<String, AgentPermissions> {
        &self.permissions
    }

    /// List all registered agent IDs that have configs.
    pub fn agent_ids(&self) -> Vec<&String> {
        self.entries.keys().collect()
    }
}
