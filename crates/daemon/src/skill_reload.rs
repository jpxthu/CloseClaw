//! Skill Registry Initialization
//!
//! Initializes the skill registry at daemon startup.
//!
//! The registry is built once at startup; no file watcher or runtime
//! rescan mechanism exists. Changes to skill files take effect at the
//! next System Prompt assembly boundary (fresh disk scan).

use closeclaw_skills::{init_disk_skills, DiskSkillRegistry, ScanConfig};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::info;

/// Initialize the skill registry at startup.
///
/// Scans skill directories once and returns the shared registry.
/// No file watcher is started; the registry is stable between
/// System Prompt assembly boundaries.
pub(crate) async fn init_skill_registry(
    config_dir: &str,
    project_root: Option<&Path>,
    extra_dirs: Vec<PathBuf>,
) -> anyhow::Result<Arc<RwLock<Option<DiskSkillRegistry>>>> {
    let agent_id = Path::new(config_dir).file_name().and_then(|s| s.to_str());
    let global_dir = derive_global_dir(config_dir);
    let config_path = Path::new(config_dir);
    let agent_skills_dir = agent_id.map(|id| {
        config_path
            .parent()
            .unwrap_or(config_path)
            .join("agents")
            .join(id)
            .join("skills")
    });
    let project_root_buf = project_root.map(|p| p.join(".closeclaw").join("skills"));
    let scan_config = build_scan_config(global_dir, agent_skills_dir, project_root_buf, extra_dirs);

    // Initialize shared registry state
    let registry = init_disk_skills(&scan_config);
    let registry_len = registry.len();
    let registry_arc = Arc::new(RwLock::new(Some(registry)));

    info!(loaded = registry_len, "skill registry initialized");

    Ok(registry_arc)
}

/// Derive the global skills directory from the config directory.
///
/// `config_dir` is typically `~/.closeclaw/<agent>`; the global
/// skills directory is `<parent>/skills` (i.e. `~/.closeclaw/skills`).
/// Returns `None` when `config_dir` has no parent (e.g. root `/`).
fn derive_global_dir(config_dir: &str) -> Option<PathBuf> {
    Path::new(config_dir).parent().map(|p| p.join("skills"))
}

/// Build a [`ScanConfig`] from the provided directories.
///
/// - `global_dir`: the global skills directory (e.g. `~/.closeclaw/skills`).
/// - `agent_skills_dir`: the per-agent skills directory.
/// - `project_root`: the project-level `.closeclaw/skills` directory.
/// - `extra_dirs`: additional directories to scan.
fn build_scan_config(
    global_dir: Option<PathBuf>,
    agent_skills_dir: Option<PathBuf>,
    project_root: Option<PathBuf>,
    extra_dirs: Vec<PathBuf>,
) -> ScanConfig {
    ScanConfig {
        global_dir,
        extra_dirs,
        agent_skills_dir,
        project_root,
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
        let expected_agent_skills = config_dir
            .parent()
            .unwrap()
            .join("agents")
            .join("my-agent")
            .join("skills");

        let scan_config = build_scan_config(
            global_dir.clone(),
            Some(expected_agent_skills.clone()),
            None,
            vec![],
        );

        assert_eq!(scan_config.global_dir, global_dir);
        assert!(scan_config.extra_dirs.is_empty());
        assert_eq!(scan_config.agent_skills_dir, Some(expected_agent_skills));
        assert!(scan_config.project_root.is_none());
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

        let scan_config = build_scan_config(None, None, None, vec![]);
        assert!(scan_config.agent_skills_dir.is_none());
        assert!(scan_config.project_root.is_none());
    }

    // --- Agent layer and Project layer scanning ---

    /// Normal path: agent_skills_dir points to parent/agents/<id>/skills.
    #[test]
    fn test_agent_skills_dir_points_to_parent_agents_id_skills() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("home/user/.closeclaw/eda");
        std::fs::create_dir_all(&config_dir).unwrap();

        // Simulate what init_skill_registry does: extract agent_id from
        // config_dir and compute the agent skills directory.
        let agent_id = config_dir.file_name().unwrap().to_str().unwrap();
        let agent_skills_dir = config_dir
            .parent()
            .unwrap()
            .join("agents")
            .join(agent_id)
            .join("skills");

        let expected = tmp.path().join("home/user/.closeclaw/agents/eda/skills");
        assert_eq!(agent_skills_dir, expected);

        let scan_config = build_scan_config(None, Some(agent_skills_dir.clone()), None, vec![]);
        assert_eq!(scan_config.agent_skills_dir, Some(expected));
    }

    /// Normal path: project_root is correctly set in ScanConfig.
    #[test]
    fn test_project_root_set_in_scan_config() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path().join("my/project");
        std::fs::create_dir_all(&project_root).unwrap();

        let scan_config = build_scan_config(None, None, Some(project_root.clone()), vec![]);
        assert_eq!(scan_config.project_root, Some(project_root));
    }

    /// Boundary: config_dir is root path (no parent) → agent_skills_dir
    /// calculation doesn't panic; parent().unwrap_or(config_path) handles it.
    #[test]
    fn test_agent_skills_dir_no_panic_when_config_dir_is_root() {
        // When config_dir is "/", file_name() is None, so agent_id = None.
        // agent_skills_dir computation is skipped (map on None).
        let agent_id = Path::new("/").file_name().and_then(|s| s.to_str());
        assert!(agent_id.is_none());

        // Simulate the compute: agent_skills_dir = None (no agent_id)
        let agent_skills_dir: Option<PathBuf> = agent_id.map(|id| {
            Path::new("/")
                .parent()
                .unwrap_or(Path::new("/"))
                .join("agents")
                .join(id)
                .join("skills")
        });
        assert!(agent_skills_dir.is_none());

        // build_scan_config should work fine with None agent_skills_dir
        let scan_config = build_scan_config(None, agent_skills_dir, None, vec![]);
        assert!(scan_config.agent_skills_dir.is_none());
    }

    /// Boundary: agent_id is None → agent_skills_dir is None.
    #[test]
    fn test_agent_skills_dir_none_when_agent_id_is_none() {
        // Explicitly pass None for agent_skills_dir to simulate the case
        // where agent_id extraction failed (e.g. config_dir has no meaningful
        // agent name component).
        let agent_skills_dir: Option<PathBuf> = None;

        let scan_config = build_scan_config(None, agent_skills_dir, None, vec![]);
        assert!(scan_config.agent_skills_dir.is_none());
    }

    /// Boundary: project_root is None → ScanConfig.project_root is None.
    #[test]
    fn test_project_root_none_when_not_provided() {
        let scan_config = build_scan_config(None, None, None, vec![]);
        assert!(scan_config.project_root.is_none());
    }

    /// Boundary: both agent_skills_dir and project_root are None.
    #[test]
    fn test_scan_config_defaults_all_none() {
        let scan_config = build_scan_config(None, None, None, vec![]);
        assert!(scan_config.global_dir.is_none());
        assert!(scan_config.agent_skills_dir.is_none());
        assert!(scan_config.project_root.is_none());
        assert!(scan_config.extra_dirs.is_empty());
    }

    // --- ExtraDirs layer ---

    /// build_scan_config correctly propagates extra_dirs into ScanConfig.
    #[test]
    fn test_build_scan_config_extra_dirs_propagated() {
        let extra = vec![
            PathBuf::from("/opt/skills"),
            PathBuf::from("/home/user/.closeclaw/extra"),
        ];
        let scan_config = build_scan_config(None, None, None, extra.clone());
        assert_eq!(scan_config.extra_dirs, extra);
    }

    /// build_scan_config: single extra_dir is preserved.
    #[test]
    fn test_build_scan_config_single_extra_dir() {
        let extra = vec![PathBuf::from("/tmp/skills")];
        let scan_config = build_scan_config(None, None, None, extra.clone());
        assert_eq!(scan_config.extra_dirs.len(), 1);
        assert_eq!(scan_config.extra_dirs[0], PathBuf::from("/tmp/skills"));
    }

    /// build_scan_config: extra_dirs coexist with other dirs.
    #[test]
    fn test_build_scan_config_extra_dirs_with_other_dirs() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("global");
        let extra = vec![PathBuf::from("/opt/skills")];
        let scan_config = build_scan_config(Some(global_dir.clone()), None, None, extra.clone());
        assert_eq!(scan_config.global_dir, Some(global_dir));
        assert_eq!(scan_config.extra_dirs, extra);
    }

    /// Normal path: all three dirs provided → all set in ScanConfig.
    #[test]
    fn test_scan_config_all_dirs_present() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("global_skills");
        let agent_dir = tmp.path().join("agent_skills");
        let project_root = tmp.path().join("project");

        let scan_config = build_scan_config(
            Some(global_dir.clone()),
            Some(agent_dir.clone()),
            Some(project_root.clone()),
            vec![],
        );
        assert_eq!(scan_config.global_dir, Some(global_dir));
        assert_eq!(scan_config.agent_skills_dir, Some(agent_dir));
        assert_eq!(scan_config.project_root, Some(project_root));
    }

    // --- init_skill_registry behavior tests ---

    /// Helper: create a minimal SKILL.md in the given directory.
    fn create_skill_in_dir(dir: &Path, name: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\ndescription: {} skill\n---\n# {}\n", name, name),
        )
        .unwrap();
    }

    /// Normal path: init_skill_registry scans the global dir and returns
    /// a populated registry.
    #[tokio::test]
    async fn test_init_skill_registry_returns_populated_registry() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("home/user/.closeclaw/eda");
        std::fs::create_dir_all(&config_dir).unwrap();

        // Create a skill in the global dir
        let global_dir = tmp.path().join("home/user/.closeclaw/skills");
        std::fs::create_dir_all(&global_dir).unwrap();
        create_skill_in_dir(&global_dir, "test-skill");

        let registry = init_skill_registry(config_dir.to_str().unwrap(), None, vec![])
            .await
            .unwrap();

        let guard = registry.read().unwrap();
        let reg = guard.as_ref().unwrap();
        assert_eq!(reg.len(), 1, "registry should contain the scanned skill");
        assert_eq!(reg.list(), vec!["test-skill"]);
    }

    /// Boundary: config_dir does not exist → init_skill_registry returns
    /// an empty registry without panicking.
    #[tokio::test]
    async fn test_init_skill_registry_empty_when_dirs_missing() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("nonexistent/.closeclaw/eda");

        let registry = init_skill_registry(config_dir.to_str().unwrap(), None, vec![])
            .await
            .unwrap();

        let guard = registry.read().unwrap();
        let reg = guard.as_ref().unwrap();
        assert_eq!(
            reg.len(),
            0,
            "registry should be empty when dirs don't exist"
        );
    }
}
