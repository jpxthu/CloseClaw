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
/// On first access for a given agent ID, reads
/// `<config_dir>/agents/<agent-id>/permissions.json` from disk and caches
/// the result. Subsequent accesses check the file's mtime; if unchanged,
/// the cached value is returned. If the mtime has changed, the file is
/// re-read.
///
/// If the file does not exist, `None` is returned (the agent has no
/// custom permission configuration).
#[derive(Debug)]
pub struct LazyAgentPermissions {
    /// The base config directory (e.g. `~/.closeclaw`).
    config_dir: PathBuf,
    /// Cache: agent_id → (permissions, file_mtime_at_load).
    cache: RwLock<HashMap<String, (AgentPermissions, SystemTime)>>,
}

impl LazyAgentPermissions {
    /// Create a new lazy permission provider rooted at `config_dir`.
    ///
    /// `config_dir` is the parent of the `agents/` directory, e.g.
    /// `~/.closeclaw`.
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Resolve the permissions.json path for a given agent.
    fn permissions_path(&self, agent_id: &str) -> PathBuf {
        self.config_dir
            .join("agents")
            .join(agent_id)
            .join("permissions.json")
    }
}

impl AgentPermissionProvider for LazyAgentPermissions {
    fn get(&self, agent_id: &str) -> Option<AgentPermissions> {
        let path = self.permissions_path(agent_id);

        // Fast path: check cache with read lock.
        {
            let cache = self.cache.read().expect("RwLock poisoned");
            if let Some((perms, cached_mtime)) = cache.get(agent_id) {
                // Check if file still exists and mtime is unchanged.
                let current_mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
                match current_mtime {
                    Some(mtime) if mtime == *cached_mtime => {
                        debug!(
                            agent_id = %agent_id,
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

        // Slow path: read from disk and update cache.
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
            cache.insert(agent_id.to_string(), (perms.clone(), mtime));
        }

        debug!(agent_id = %agent_id, "loaded permissions from disk");
        Some(perms)
    }
}
