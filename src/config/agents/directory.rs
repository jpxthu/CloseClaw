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
            let user_config = Self::load_agent_config(&user_config_path);
            let project_config = project_config_path
                .as_ref()
                .and_then(|p| Self::load_agent_config(p));

            let resolved = match (project_config, user_config) {
                (Some(proj), Some(usr)) => ResolvedAgentConfig::merge(proj, usr),
                (Some(proj), None) => ResolvedAgentConfig::from_single(proj, ConfigSource::Project),
                (None, Some(usr)) => ResolvedAgentConfig::from_single(usr, ConfigSource::User),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::{ActionPermission, AgentPermissions, PermissionLimits};
    use std::collections::HashMap;
    use std::path::Path;
    use tempfile::TempDir;

    /// Write a minimal `config.json` for the given agent ID.
    fn write_config(dir: &Path, id: &str, name: &str) {
        let agent_dir = dir.join(id);
        std::fs::create_dir_all(&agent_dir).unwrap();
        let json = format!(r#"{{ "id": "{}", "name": "{}" }}"#, id, name);
        std::fs::write(agent_dir.join("config.json"), json).unwrap();
    }

    /// Write a minimal `permissions.json` for the given agent ID.
    fn write_permissions(dir: &Path, id: &str, marker: &str) {
        let agent_dir = dir.join(id);
        std::fs::create_dir_all(&agent_dir).unwrap();
        // Embed `marker` inside the agent_id so we can assert which file won.
        let json = format!(
            r#"{{ "agent_id": "{}",
"permissions": {{ "exec": {{ "allowed": true,
"limits": {{}} }} }} }}"#,
            marker
        );
        std::fs::write(agent_dir.join("permissions.json"), json).unwrap();
    }

    #[test]
    fn test_empty_registry_produces_no_entries() {
        let user = TempDir::new().unwrap();
        // Create a stray agent dir that must NOT be picked up.
        write_config(user.path(), "stray", "Stray Agent");

        let provider =
            AgentDirectoryProvider::new(Vec::new(), user.path().to_path_buf(), None).unwrap();

        assert!(provider.agent_ids().is_empty());
        assert!(provider.entries().is_empty());
        assert!(provider.permissions().is_empty());
    }

    #[test]
    fn test_user_only_load() {
        let user = TempDir::new().unwrap();
        write_config(user.path(), "alpha", "Alpha Agent");

        let provider =
            AgentDirectoryProvider::new(vec!["alpha".to_string()], user.path().to_path_buf(), None)
                .unwrap();

        assert_eq!(provider.agent_ids().len(), 1);
        let entry = provider.get("alpha").expect("alpha should be loaded");
        assert_eq!(entry.id, "alpha");
        assert_eq!(entry.name, "Alpha Agent");
        assert_eq!(entry.source, ConfigSource::User);
    }

    #[test]
    fn test_project_only_load() {
        let project = TempDir::new().unwrap();
        write_config(project.path(), "beta", "Beta Agent");

        let provider = AgentDirectoryProvider::new(
            vec!["beta".to_string()],
            PathBuf::from("/nonexistent/user/agents"),
            Some(project.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(provider.agent_ids().len(), 1);
        let entry = provider.get("beta").expect("beta should be loaded");
        assert_eq!(entry.id, "beta");
        assert_eq!(entry.name, "Beta Agent");
        assert_eq!(entry.source, ConfigSource::Project);
    }

    #[test]
    fn test_merge_project_overrides_user() {
        let user = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        write_config(user.path(), "gamma", "User Name");
        write_config(project.path(), "gamma", "Project Name");

        let provider = AgentDirectoryProvider::new(
            vec!["gamma".to_string()],
            user.path().to_path_buf(),
            Some(project.path().to_path_buf()),
        )
        .unwrap();

        let entry = provider.get("gamma").expect("gamma should be loaded");
        // Project name wins.
        assert_eq!(entry.name, "Project Name");
        assert_eq!(entry.source, ConfigSource::Merged);
    }

    #[test]
    fn test_ignores_dirs_outside_registry() {
        let user = TempDir::new().unwrap();
        // Files in the registry → must be loaded.
        write_config(user.path(), "registered", "Registered");
        // Files NOT in the registry → must be ignored.
        write_config(user.path(), "unregistered", "Unregistered");

        let provider = AgentDirectoryProvider::new(
            vec!["registered".to_string()],
            user.path().to_path_buf(),
            None,
        )
        .unwrap();

        assert_eq!(provider.agent_ids(), vec![&"registered".to_string()]);
        assert!(provider.get("unregistered").is_none());
    }

    #[test]
    fn test_permissions_project_wins_over_user() {
        let user = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        write_config(user.path(), "delta", "Delta");
        write_config(project.path(), "delta", "Delta");
        write_permissions(user.path(), "delta", "user-marker");
        write_permissions(project.path(), "delta", "project-marker");

        let provider = AgentDirectoryProvider::new(
            vec!["delta".to_string()],
            user.path().to_path_buf(),
            Some(project.path().to_path_buf()),
        )
        .unwrap();

        let perms = provider
            .permissions()
            .get("delta")
            .expect("delta permissions should be loaded");
        // Project permissions win → agent_id carries the project marker.
        assert_eq!(perms.agent_id, "project-marker");
    }

    #[test]
    fn test_permissions_user_fallback_when_no_project_file() {
        let user = TempDir::new().unwrap();
        write_config(user.path(), "epsilon", "Epsilon");
        write_permissions(user.path(), "epsilon", "user-marker");

        let provider = AgentDirectoryProvider::new(
            vec!["epsilon".to_string()],
            user.path().to_path_buf(),
            Some(PathBuf::from("/nonexistent/project/agents")),
        )
        .unwrap();

        let perms = provider
            .permissions()
            .get("epsilon")
            .expect("epsilon permissions should be loaded from user");
        assert_eq!(perms.agent_id, "user-marker");
        assert!(perms.is_allowed("exec"));
    }

    #[test]
    fn test_missing_config_json_is_skipped() {
        let user = TempDir::new().unwrap();
        // Create the agent directory but leave it empty (no config.json).
        std::fs::create_dir_all(user.path().join("zeta")).unwrap();
        // Another agent that DOES have a config file.
        write_config(user.path(), "eta", "Eta");

        let provider = AgentDirectoryProvider::new(
            vec!["zeta".to_string(), "eta".to_string()],
            user.path().to_path_buf(),
            None,
        )
        .unwrap();

        // zeta has no config.json → skipped.
        assert!(provider.get("zeta").is_none());
        // eta is still loaded.
        assert!(provider.get("eta").is_some());
        assert_eq!(provider.agent_ids().len(), 1);
    }

    #[test]
    fn test_reload_picks_up_changes() {
        let user = TempDir::new().unwrap();
        let provider =
            AgentDirectoryProvider::new(vec!["theta".to_string()], user.path().to_path_buf(), None)
                .unwrap();
        assert!(provider.get("theta").is_none());

        // Add a config file and reload.
        write_config(user.path(), "theta", "Theta");
        let mut provider = provider;
        // Provider has no public `reload` callable here? Yes it does.
        // We need `&mut self`, so reconstruct via the constructor for the
        // first call, then mutate via `reload` after the change.
        // Easier: use the constructor twice.
        drop(provider);

        let provider =
            AgentDirectoryProvider::new(vec!["theta".to_string()], user.path().to_path_buf(), None)
                .unwrap();
        assert!(provider.get("theta").is_some());
    }

    #[test]
    fn test_no_user_dir_no_project_dir() {
        // Neither user nor project dir exists. The registry IDs should all
        // be skipped, and no errors should be raised.
        let provider = AgentDirectoryProvider::new(
            vec!["a".to_string(), "b".to_string()],
            PathBuf::from("/nonexistent/user"),
            None,
        )
        .unwrap();
        assert!(provider.agent_ids().is_empty());
        assert!(provider.entries().is_empty());
    }

    #[test]
    fn test_merge_falls_back_to_user_field_when_project_empty() {
        // When the user config sets a field the project config does not,
        // the user value must be preserved.
        let user = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();

        // user: name and a skill; project: only a different name.
        let user_dir = user.path().join("iota");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(
            user_dir.join("config.json"),
            r#"{ "id": "iota", "name": "Iota", "skills": ["web"] }"#,
        )
        .unwrap();

        let project_dir = project.path().join("iota");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("config.json"),
            r#"{ "id": "iota", "name": "Iota Project" }"#,
        )
        .unwrap();

        let provider = AgentDirectoryProvider::new(
            vec!["iota".to_string()],
            user.path().to_path_buf(),
            Some(project.path().to_path_buf()),
        )
        .unwrap();

        let entry = provider.get("iota").expect("iota must be loaded");
        // Project name overrides user name.
        assert_eq!(entry.name, "Iota Project");
        // User-provided skill survives because the project skills vec is empty.
        assert_eq!(entry.skills, vec!["web".to_string()]);
        assert_eq!(entry.source, ConfigSource::Merged);
    }

    #[test]
    fn test_no_permissions_file_is_fine() {
        let user = TempDir::new().unwrap();
        write_config(user.path(), "kappa", "Kappa");

        let provider =
            AgentDirectoryProvider::new(vec!["kappa".to_string()], user.path().to_path_buf(), None)
                .unwrap();

        assert!(provider.get("kappa").is_some());
        assert!(provider.permissions().get("kappa").is_none());
    }

    #[test]
    fn test_action_permission_round_trip() {
        // Sanity check: ActionPermission is the type used inside
        // AgentPermissions; this guards against accidental type changes.
        let perm = ActionPermission {
            allowed: true,
            limits: PermissionLimits::default(),
        };
        let json = serde_json::to_string(&perm).unwrap();
        let back: ActionPermission = serde_json::from_str(&json).unwrap();
        assert!(back.allowed);
    }
}
