//! Skill Hot Reload Initialization
//!
//! Initializes the skill registry and file watcher at daemon startup.

use closeclaw_skills::{
    init_disk_skills, start_skill_watcher, DiskSkillRegistry, ScanConfig, SkillWatcherHandle,
};
use closeclaw_system_prompt::sections::invalidate_skill_listing;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::info;

/// Initialize skill hot reload system.
///
/// Scans configured skill directories and starts a file watcher.
/// When skill files change, the registry is re-scanned and cached
/// listings are invalidated.
///
/// Returns the shared skill registry and the watcher handle
/// (RAII: stops on drop).
pub(crate) async fn init_skill_hot_reload(
    config_dir: &str,
) -> anyhow::Result<(
    Arc<RwLock<Option<DiskSkillRegistry>>>,
    Option<SkillWatcherHandle>,
)> {
    let global_dir = derive_global_dir(config_dir);
    let scan_config = build_scan_config(global_dir.clone(), Path::new(config_dir), None);
    let skill_dirs = build_skill_dirs(global_dir);

    // Initialize shared registry state
    let registry = init_disk_skills(&scan_config);
    let registry_len = registry.len();
    let registry_arc = Arc::new(RwLock::new(Some(registry)));
    let registry_for_watcher = Arc::clone(&registry_arc);

    info!(loaded = registry_len, "skill registry initialized");

    // Start watcher — re-scan uses the same ScanConfig as initial scan
    let watcher_config = scan_config.clone();
    let watcher = if skill_dirs.is_empty() {
        info!("no skill directories to watch, skipping hot reload watcher");
        None
    } else {
        Some(start_skill_watcher(
            skill_dirs,
            Box::new(move || {
                let mut new_registry = init_disk_skills(&watcher_config);

                // Preserve the AgentRegistry reference from the old registry
                // so the Skills Registry can continue querying agent configs
                // directly after hot-reload.
                if let Ok(guard) = registry_for_watcher.read() {
                    if let Some(ref old_reg) = *guard {
                        if let Some(agent_reg) = old_reg.agent_skills_query() {
                            new_registry.set_agent_skills_query(Arc::clone(agent_reg));
                        }
                    }
                }

                // Update shared state
                if let Ok(mut guard) = registry_for_watcher.write() {
                    *guard = Some(new_registry);
                }

                // Invalidate cache so next build picks up new listing
                invalidate_skill_listing();

                tracing::info!("skill registry reloaded after file change");
            }),
        )?)
    };

    info!("skill hot reload initialized");
    Ok((registry_arc, watcher))
}

/// Derive the global skills directory from the config directory.
///
/// `config_dir` is typically `~/.closeclaw/<agent>`; the global
/// skills directory is `<parent>/skills` (i.e. `~/.closeclaw/skills`).
/// Returns `None` when `config_dir` has no parent (e.g. root `/`).
fn derive_global_dir(config_dir: &str) -> Option<PathBuf> {
    Path::new(config_dir).parent().map(|p| p.join("skills"))
}

/// Build the list of directories to watch for skill changes.
///
/// Includes `global_dir` only when it exists on disk.
fn build_skill_dirs(global_dir: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Some(gd) = global_dir {
        if gd.exists() {
            dirs.push(gd);
        }
    }
    dirs
}

/// Build a [`ScanConfig`] for the given global directory.
///
/// `config_dir` is the root config directory (e.g. `~/.closeclaw`).
/// `agent_id` is the agent identifier; when provided, `agent_skills_dir`
/// is derived as `{config_dir}/agents/{agent_id}/skills/`.
fn build_scan_config(
    global_dir: Option<PathBuf>,
    config_dir: &Path,
    agent_id: Option<&str>,
) -> ScanConfig {
    let agent_skills_dir = agent_id.map(|id| config_dir.join("agents").join(id).join("skills"));
    ScanConfig {
        global_dir,
        extra_dirs: vec![],
        agent_skills_dir,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_global_dir_derived_from_config_dir_parent() {
        // Create a temp dir structure: <tmp>/home/user/.closeclaw/eda
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("home/user/.closeclaw/eda");
        std::fs::create_dir_all(&config_dir).unwrap();

        let result = derive_global_dir(config_dir.to_str().unwrap());
        let expected = tmp.path().join("home/user/.closeclaw/skills");
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn test_scan_config_contains_global_dir() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("home/user/.closeclaw/eda");
        std::fs::create_dir_all(&config_dir).unwrap();

        let global_dir = derive_global_dir(config_dir.to_str().unwrap());

        let scan_config =
            build_scan_config(global_dir.clone(), config_dir.as_path(), Some("my-agent"));

        assert_eq!(scan_config.global_dir, global_dir);
        assert!(scan_config.extra_dirs.is_empty());
        assert_eq!(
            scan_config.agent_skills_dir,
            Some(config_dir.join("agents").join("my-agent").join("skills"))
        );
    }

    #[test]
    fn test_skill_dirs_contains_global_only_when_exists() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("home/user/.closeclaw/eda");
        std::fs::create_dir_all(&config_dir).unwrap();

        let global_dir = derive_global_dir(config_dir.to_str().unwrap()).unwrap();
        std::fs::create_dir_all(&global_dir).unwrap();

        let skill_dirs = build_skill_dirs(Some(global_dir.clone()));

        assert_eq!(skill_dirs.len(), 1);
        assert!(skill_dirs.contains(&global_dir));
    }

    #[test]
    fn test_global_dir_none_when_no_parent() {
        let result = derive_global_dir("/");
        assert_eq!(result, None);
    }

    #[test]
    fn test_agent_skills_dir_none_without_agent_id() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("home/user/.closeclaw");
        std::fs::create_dir_all(&config_dir).unwrap();

        let scan_config = build_scan_config(None, config_dir.as_path(), None);
        assert!(scan_config.agent_skills_dir.is_none());
    }
}
