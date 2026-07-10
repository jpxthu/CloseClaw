//! Lazy agent permission provider.
//!
//! Implements the design doc's "rule loading strategy":
//! Agent-dimension permission rules are lazily loaded on first `evaluate()`
//! query, not eagerly scanned at init. Files are cached with mtime tracking
//! for automatic invalidation.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::SystemTime;

use tracing::{debug, warn};

use super::config_types::AgentPermissions;

/// Trait for lazily providing agent permissions by agent ID.
///
/// Implementations may load from disk, cache, or any other source.
pub trait AgentPermissionProvider: Send + Sync {
    /// Get the permissions for the given agent, or `None` if the agent
    /// has no custom permission configuration.
    fn get(&self, agent_id: &str) -> Option<AgentPermissions>;
}

/// No-op permission provider used as a default when no real provider is configured.
///
/// Returns `None` for all agent IDs.
pub struct NoopPermissionProvider;

impl AgentPermissionProvider for NoopPermissionProvider {
    fn get(&self, _agent_id: &str) -> Option<AgentPermissions> {
        None
    }
}

/// Lazy-loading agent permission provider.
///
/// On first access for a given agent ID, checks two paths in priority order:
/// 1. Project-level: `<project_agents_dir>/<agent-id>/permissions.json`
///    (highest priority — when present, used exclusively)
/// 2. User-level: `<config_dir>/agents/<agent-id>/permissions.json`
///    (fallback when project-level does not exist)
///
/// If neither file exists, `None` is returned. Results are cached with
/// mtime tracking for automatic invalidation.
#[derive(Debug)]
pub struct LazyAgentPermissions {
    /// The base config directory (e.g. `~/.closeclaw`).
    config_dir: PathBuf,
    /// Optional project-level agents directory (e.g. `<repo>/.closeclaw/agents`).
    /// When set, project-level permissions take priority over user-level.
    project_agents_dir: RwLock<Option<PathBuf>>,
    /// Cache: agent_id → (permissions, file_mtime_at_load, resolved_path).
    cache: RwLock<HashMap<String, (AgentPermissions, SystemTime, PathBuf)>>,
}

impl LazyAgentPermissions {
    /// Create a new lazy permission provider rooted at `config_dir`.
    ///
    /// `config_dir` is the parent of the `agents/` directory, e.g.
    /// `~/.closeclaw`.
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            project_agents_dir: RwLock::new(None),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Set the project-level agents directory for priority loading.
    ///
    /// `project_agents_dir` is the `<repo>/.closeclaw/agents` directory.
    /// When set, project-level `permissions.json` takes priority over
    /// user-level for any given agent.
    pub fn set_project_agents_dir(&self, project_agents_dir: PathBuf) {
        *self.project_agents_dir.write().expect("RwLock poisoned") = Some(project_agents_dir);
    }

    /// Resolve the user-level permissions.json path for a given agent.
    fn user_permissions_path(&self, agent_id: &str) -> PathBuf {
        self.config_dir
            .join("agents")
            .join(agent_id)
            .join("permissions.json")
    }

    /// Resolve the project-level permissions.json path for a given agent.
    fn project_permissions_path(&self, agent_id: &str) -> Option<PathBuf> {
        self.project_agents_dir
            .read()
            .expect("RwLock poisoned")
            .as_ref()
            .map(|d| d.join(agent_id).join("permissions.json"))
    }

    /// Resolve which permissions.json to use, applying priority rules:
    /// project-level > user-level.
    fn resolve_path(&self, agent_id: &str) -> Option<PathBuf> {
        // Check project-level first (highest priority).
        if let Some(proj_path) = self.project_permissions_path(agent_id) {
            if proj_path.exists() {
                return Some(proj_path);
            }
        }
        // Fall back to user-level.
        let user_path = self.user_permissions_path(agent_id);
        if user_path.exists() {
            Some(user_path)
        } else {
            None
        }
    }
}

impl AgentPermissionProvider for LazyAgentPermissions {
    fn get(&self, agent_id: &str) -> Option<AgentPermissions> {
        // Fast path: check cache with read lock.
        {
            let cache = self.cache.read().expect("RwLock poisoned");
            if let Some((perms, cached_mtime, cached_path)) = cache.get(agent_id) {
                // Check if the cached file still exists and mtime is unchanged.
                let current_mtime = fs::metadata(cached_path)
                    .ok()
                    .and_then(|m| m.modified().ok());
                match current_mtime {
                    Some(mtime) if mtime == *cached_mtime => {
                        debug!(
                            agent_id = %agent_id,
                            path = %cached_path.display(),
                            "permission cache hit (mtime unchanged)"
                        );
                        return Some(perms.clone());
                    }
                    _ => {
                        debug!(
                            agent_id = %agent_id,
                            "permission cache stale or file gone, re-loading"
                        );
                    }
                }
            }
        }

        // Slow path: resolve path with priority, read from disk, and cache.
        let path = self.resolve_path(agent_id)?;
        let content = fs::read_to_string(&path).ok()?;
        let perms: AgentPermissions = match serde_json::from_str(&content) {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    agent_id = %agent_id,
                    path = %path.display(),
                    error = %e,
                    "failed to parse permissions.json"
                );
                return None;
            }
        };
        let mtime = fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        {
            let mut cache = self.cache.write().expect("RwLock poisoned");
            cache.insert(agent_id.to_string(), (perms.clone(), mtime, path.clone()));
        }

        debug!(agent_id = %agent_id, path = %path.display(), "loaded permissions from disk");
        Some(perms)
    }
}
