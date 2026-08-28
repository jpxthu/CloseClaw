//! Skill Registry Initialization
//!
//! Initializes the skill registry at daemon startup.
//!
//! The shared rescan handle allows the admin RPC `skill rescan` command
//! to trigger an immediate registry rebuild without relying on a file watcher.

use closeclaw_skills::{init_disk_skills, DiskSkillRegistry, ScanConfig};
use closeclaw_system_prompt::sections::SectionCache;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::info;

/// Shared rescan handle for `skill rescan` admin command.
///
/// Encapsulates the scan configuration and shared state references needed
/// to perform an immediate skill registry rescan on demand. The admin server
/// invokes `perform()` when handling a `SkillRescan` RPC request.
pub(crate) struct SkillRescanHandle {
    registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
    scan_config: ScanConfig,
    shared_cache: Arc<RwLock<SectionCache>>,
}

impl SkillRescanHandle {
    fn new(
        registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
        scan_config: ScanConfig,
        shared_cache: Arc<RwLock<SectionCache>>,
    ) -> Self {
        Self {
            registry,
            scan_config,
            shared_cache,
        }
    }

    /// Trigger an immediate skill registry rescan.
    ///
    /// Rebuilds the registry from disk, preserves the `agent_skills_query`
    /// reference from the previous registry, and invalidates the
    /// `skill_listing` cache so the next system prompt build picks up
    /// changes.
    pub(crate) fn perform(&self) {
        perform_skill_rescan(&self.registry, &self.scan_config, &self.shared_cache);
    }
}

/// Re-scan skill directories and update the shared registry + cache.
///
/// This is the core rescan logic used by the `skill rescan` admin command.
/// It rebuilds the registry from disk, preserves the `agent_skills_query`
/// reference from the old registry, and invalidates the `skill_listing`
/// section cache.
pub(crate) fn perform_skill_rescan(
    registry: &Arc<RwLock<Option<DiskSkillRegistry>>>,
    scan_config: &ScanConfig,
    shared_cache: &Arc<RwLock<SectionCache>>,
) {
    let mut new_registry = init_disk_skills(scan_config);

    // Preserve the AgentRegistry reference from the old registry
    // so the Skills Registry can continue querying agent configs
    // directly after rescan.
    if let Ok(guard) = registry.read() {
        if let Some(ref old_reg) = *guard {
            if let Some(agent_reg) = old_reg.agent_skills_query() {
                new_registry.set_agent_skills_query(Arc::clone(agent_reg));
            }
        }
    }

    // Update shared state
    if let Ok(mut guard) = registry.write() {
        *guard = Some(new_registry);
    }

    // Invalidate cache so next build picks up new listing
    shared_cache.write().unwrap().invalidate_skill_listing();

    tracing::info!("skill registry rescan complete");
}

/// Initialize the skill registry and rescan handle.
///
/// Scans skill directories once at startup and builds a [`SkillRescanHandle`]
/// for the admin RPC `skill rescan` command. No file watcher is started;
/// rescan is triggered on demand.
///
/// Returns the shared skill registry and the rescan handle.
pub(crate) async fn init_skill_registry(
    config_dir: &str,
    project_root: Option<&Path>,
    shared_cache: Arc<RwLock<SectionCache>>,
    extra_dirs: Vec<PathBuf>,
) -> anyhow::Result<(Arc<RwLock<Option<DiskSkillRegistry>>>, SkillRescanHandle)> {
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

    // Build the rescan handle for admin RPC.
    let rescan_handle = SkillRescanHandle::new(
        Arc::clone(&registry_arc),
        scan_config,
        Arc::clone(&shared_cache),
    );
    Ok((registry_arc, rescan_handle))
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

    // --- Step 1.3 tests: Agent layer and Project layer scanning ---

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

    // --- Step 1.3 tests: ExtraDirs layer ---

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

    // --- Rescan behavior tests (perform_skill_rescan) ---

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

    /// Helper: build a mock AgentSkillsQuery that returns the given skills list.
    struct MockAgentSkillsQuery {
        skills: Vec<String>,
    }

    impl closeclaw_common::AgentSkillsQuery for MockAgentSkillsQuery {
        fn get_agent_skills(&self, _agent_id: &str) -> Option<Vec<String>> {
            Some(self.skills.clone())
        }
    }

    /// Normal path: rescan picks up a skill added after initial scan.
    ///
    /// Verifies that after perform_skill_rescan, a newly created skill
    /// directory (with SKILL.md) appears in the registry's skill list.
    #[test]
    fn test_perform_skill_rescan_rebuilds_registry_with_new_skill() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&global_dir).unwrap();

        let scan_config = ScanConfig {
            global_dir: Some(global_dir.clone()),
            ..Default::default()
        };

        let shared_cache = Arc::new(RwLock::new(SectionCache::new()));

        // Initial scan: no skills yet
        let initial_registry = init_disk_skills(&scan_config);
        assert_eq!(initial_registry.len(), 0);
        let registry = Arc::new(RwLock::new(Some(initial_registry)));

        // Add a skill to the directory AFTER initial scan
        create_skill_in_dir(&global_dir, "new-skill");

        // Perform rescan
        perform_skill_rescan(&registry, &scan_config, &shared_cache);

        // Verify the new skill is now in the registry
        let guard = registry.read().unwrap();
        let reg = guard.as_ref().unwrap();
        assert_eq!(reg.len(), 1, "rescan should pick up newly added skill");
        assert_eq!(reg.list(), vec!["new-skill"]);
    }

    /// Normal path: rescan preserves the agent_skills_query reference.
    ///
    /// Verifies that after perform_skill_rescan, the agent_skills_query
    /// set on the old registry is carried over to the new registry.
    #[test]
    fn test_perform_skill_rescan_preserves_agent_skills_query() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&global_dir).unwrap();

        let scan_config = ScanConfig {
            global_dir: Some(global_dir.clone()),
            ..Default::default()
        };
        let shared_cache = Arc::new(RwLock::new(SectionCache::new()));

        // Create initial registry with agent_skills_query set
        let mut initial_registry = init_disk_skills(&scan_config);
        let mock_query: Arc<dyn closeclaw_common::AgentSkillsQuery> =
            Arc::new(MockAgentSkillsQuery {
                skills: vec!["skill-a".to_string()],
            });
        initial_registry.set_agent_skills_query(Arc::clone(&mock_query));
        let registry = Arc::new(RwLock::new(Some(initial_registry)));

        // Perform rescan (no skills on disk)
        perform_skill_rescan(&registry, &scan_config, &shared_cache);

        // Verify agent_skills_query is preserved
        let guard = registry.read().unwrap();
        let reg = guard.as_ref().unwrap();
        assert!(
            reg.agent_skills_query().is_some(),
            "agent_skills_query should survive rescan"
        );
        // Verify the query still works
        let result = reg
            .agent_skills_query()
            .unwrap()
            .get_agent_skills("test-agent");
        assert_eq!(result, Some(vec!["skill-a".to_string()]));
    }

    /// Normal path: rescan invalidates the skill_listing cache.
    ///
    /// Verifies that after perform_skill_rescan, the skill_listing
    /// section is removed from the shared SectionCache.
    #[test]
    fn test_perform_skill_rescan_invalidates_skill_listing_cache() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&global_dir).unwrap();

        let scan_config = ScanConfig {
            global_dir: Some(global_dir.clone()),
            ..Default::default()
        };
        let shared_cache = Arc::new(RwLock::new(SectionCache::new()));

        // Populate the skill_listing cache entry
        {
            let mut cache = shared_cache.write().unwrap();
            cache.put("skill_listing", "old listing".to_string(), None);
        }
        // Confirm cache entry exists
        assert!(
            shared_cache
                .read()
                .unwrap()
                .get("skill_listing", None)
                .is_some(),
            "skill_listing should be cached before rescan"
        );

        let initial_registry = init_disk_skills(&scan_config);
        let registry = Arc::new(RwLock::new(Some(initial_registry)));

        // Perform rescan
        perform_skill_rescan(&registry, &scan_config, &shared_cache);

        // Verify skill_listing cache is invalidated
        assert!(
            shared_cache
                .read()
                .unwrap()
                .get("skill_listing", None)
                .is_none(),
            "skill_listing cache should be invalidated after rescan"
        );
    }

    /// State transition: rescan on empty dir replaces old registry.
    ///
    /// Verifies that the old registry (potentially containing skills)
    /// is completely replaced by the rescan result.
    #[test]
    fn test_perform_skill_rescan_replaces_old_registry() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&global_dir).unwrap();

        let scan_config = ScanConfig {
            global_dir: Some(global_dir.clone()),
            ..Default::default()
        };
        let shared_cache = Arc::new(RwLock::new(SectionCache::new()));

        // Add a skill, scan, then remove it
        create_skill_in_dir(&global_dir, "old-skill");
        let initial_registry = init_disk_skills(&scan_config);
        assert_eq!(initial_registry.len(), 1);
        let registry = Arc::new(RwLock::new(Some(initial_registry)));

        // Remove the skill from disk
        std::fs::remove_dir_all(global_dir.join("old-skill")).unwrap();

        // Rescan should reflect the empty state
        perform_skill_rescan(&registry, &scan_config, &shared_cache);

        let guard = registry.read().unwrap();
        let reg = guard.as_ref().unwrap();
        assert_eq!(
            reg.len(),
            0,
            "rescan should replace old registry with empty one"
        );
    }

    /// SkillRescanHandle::perform delegates to perform_skill_rescan.
    ///
    /// Verifies that the handle correctly triggers a rescan and the
    /// result is reflected in the shared registry.
    #[test]
    fn test_rescan_handle_perform_triggers_rescan() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&global_dir).unwrap();

        let scan_config = ScanConfig {
            global_dir: Some(global_dir.clone()),
            ..Default::default()
        };
        let shared_cache = Arc::new(RwLock::new(SectionCache::new()));

        let initial_registry = init_disk_skills(&scan_config);
        let registry = Arc::new(RwLock::new(Some(initial_registry)));

        let handle = SkillRescanHandle::new(
            Arc::clone(&registry),
            scan_config.clone(),
            Arc::clone(&shared_cache),
        );

        // Add a skill after initial scan
        create_skill_in_dir(&global_dir, "handled-skill");

        // Trigger rescan via handle
        handle.perform();

        let guard = registry.read().unwrap();
        let reg = guard.as_ref().unwrap();
        assert_eq!(reg.list(), vec!["handled-skill"]);
    }
}
