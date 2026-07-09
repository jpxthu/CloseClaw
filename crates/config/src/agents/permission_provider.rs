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

use tracing::debug;

use super::config_types::AgentPermissions;

/// Trait for lazily providing agent permissions by agent ID.
///
/// Implementations may load from disk, cache, or any other source.
pub trait AgentPermissionProvider: Send + Sync {
    /// Get the permissions for the given agent, or `None` if the agent
    /// has no custom permission configuration.
    fn get(&self, agent_id: &str) -> Option<AgentPermissions>;
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
        let perms: AgentPermissions = serde_json::from_str(&content).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::FileTime;
    use std::fs;
    use tempfile::TempDir;

    fn make_perms(agent_id: &str) -> AgentPermissions {
        let mut permissions = HashMap::new();
        permissions.insert(
            "file_read".to_string(),
            super::super::config_types::ActionPermission {
                allowed: true,
                limits: Default::default(),
            },
        );
        AgentPermissions {
            agent_id: agent_id.to_string(),
            permissions,
            inherited_from: None,
        }
    }

    #[test]
    fn test_get_returns_permissions_when_file_exists() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agents").join("test-agent");
        fs::create_dir_all(&agent_dir).unwrap();
        let perms = make_perms("test-agent");
        let json = serde_json::to_string(&perms).unwrap();
        fs::write(agent_dir.join("permissions.json"), json).unwrap();

        let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());
        let result = provider.get("test-agent");
        assert!(result.is_some());
        let loaded = result.unwrap();
        assert_eq!(loaded.agent_id, "test-agent");
        assert!(loaded.is_allowed("file_read"));
    }

    #[test]
    fn test_get_returns_none_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());
        assert!(provider.get("nonexistent").is_none());
    }

    #[test]
    fn test_cache_hit_on_unchanged_mtime() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agents").join("cached-agent");
        fs::create_dir_all(&agent_dir).unwrap();
        let perms = make_perms("cached-agent");
        let json = serde_json::to_string(&perms).unwrap();
        fs::write(agent_dir.join("permissions.json"), &json).unwrap();

        let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());

        // First call loads from disk.
        let first = provider.get("cached-agent").unwrap();
        assert_eq!(first.agent_id, "cached-agent");

        // Second call should use cache (same mtime).
        let second = provider.get("cached-agent").unwrap();
        assert_eq!(second.agent_id, "cached-agent");
    }

    #[test]
    fn test_cache_invalidation_on_file_replacement() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agents").join("mut-agent");
        fs::create_dir_all(&agent_dir).unwrap();
        let path = agent_dir.join("permissions.json");

        let perms1 = make_perms("mut-agent");
        fs::write(&path, serde_json::to_string(&perms1).unwrap()).unwrap();

        let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());
        let first = provider.get("mut-agent").unwrap();
        assert!(first.is_allowed("file_read"));

        // Rewrite the file with different content and explicitly advance the
        // mtime so the cache is reliably invalidated even on fast machines.
        let mut perms2 = make_perms("mut-agent");
        perms2.permissions.insert(
            "file_read".to_string(),
            super::super::config_types::ActionPermission {
                allowed: false,
                limits: Default::default(),
            },
        );
        fs::write(&path, serde_json::to_string(&perms2).unwrap()).unwrap();
        // Advance mtime by 10 seconds to guarantee cache invalidation.
        let now = FileTime::now();
        let new_mtime = FileTime::from_unix_time(now.unix_seconds() + 10, now.nanoseconds());
        filetime::set_file_mtime(&path, new_mtime).unwrap();

        let second = provider.get("mut-agent").unwrap();
        assert!(!second.is_allowed("file_read"));
    }

    #[test]
    fn test_json_parse_error_returns_none() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agents").join("bad-json");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("permissions.json"), "not valid json {{{").unwrap();

        let provider = LazyAgentPermissions::new(tmp.path().to_path_buf());
        assert!(provider.get("bad-json").is_none());
    }
}
