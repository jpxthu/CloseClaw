//! Skill Hot Reload Initialization
//!
//! Initializes the skill registry and file watcher at daemon startup.
//!
//! Implements the design doc's "file change" trigger for incremental
//! skill listing updates (`docs/design/skills/skill-listing-injection.md`):
//! file changes → re-scan registry → invalidate listing cache → next turn
//! the Session module picks up the updated listing.

use closeclaw_skills::{
    init_disk_skills, start_skill_watcher, DiskSkillRegistry, ScanConfig, SkillWatcherHandle,
};
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
/// This is the core rescan logic shared between the file-watcher hot-reload
/// path and the `skill rescan` admin command. It rebuilds the registry from
/// disk, preserves the `agent_skills_query` reference from the old registry,
/// and invalidates the `skill_listing` section cache.
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

/// Initialize skill hot reload system.
///
/// Implements the design doc's file-change-driven hot reload path:
/// "SKILL.md create/modify/delete → 300ms debounce → invalidate
/// listing cache → re-scan changed directory, update registry
/// listing cache → next turn update attachment content."
///
/// When skill files change, the watcher callback re-scans the
/// registry and invalidates the `skill_listing` entry on the shared
/// [`SectionCache`], so the next system prompt build regenerates
/// from the current registry state.
///
/// Returns the shared skill registry and the watcher handle
/// (RAII: stops on drop).
pub(crate) async fn init_skill_hot_reload(
    config_dir: &str,
    project_root: Option<&Path>,
    shared_cache: Arc<RwLock<SectionCache>>,
    extra_dirs: Vec<PathBuf>,
) -> anyhow::Result<(
    Arc<RwLock<Option<DiskSkillRegistry>>>,
    Option<SkillWatcherHandle>,
    SkillRescanHandle,
)> {
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
    let scan_config = build_scan_config(
        global_dir.clone(),
        agent_skills_dir.clone(),
        project_root_buf.clone(),
        extra_dirs.clone(),
    );
    let skill_dirs = build_skill_dirs(global_dir, agent_skills_dir, project_root_buf, extra_dirs);

    // Initialize shared registry state
    let registry = init_disk_skills(&scan_config);
    let registry_len = registry.len();
    let registry_arc = Arc::new(RwLock::new(Some(registry)));

    info!(loaded = registry_len, "skill registry initialized");

    // Build the rescan handle — shared between watcher and admin RPC.
    let rescan_handle = SkillRescanHandle::new(
        Arc::clone(&registry_arc),
        scan_config.clone(),
        Arc::clone(&shared_cache),
    );

    // Start watcher — re-scan uses the same ScanConfig as initial scan
    let watcher_config = scan_config;
    let registry_for_watcher = Arc::clone(&registry_arc);
    let watcher = if skill_dirs.is_empty() {
        info!("no skill directories to watch, skipping hot reload watcher");
        None
    } else {
        Some(start_skill_watcher(
            skill_dirs,
            Box::new(move || {
                perform_skill_rescan(&registry_for_watcher, &watcher_config, &shared_cache);
                tracing::info!("skill registry reloaded after file change");
            }),
        )?)
    };

    info!("skill hot reload initialized");
    Ok((registry_arc, watcher, rescan_handle))
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
/// `extra_dirs` directories are appended when they exist on disk.
fn build_skill_dirs(
    global_dir: Option<PathBuf>,
    agent_skills_dir: Option<PathBuf>,
    project_root: Option<PathBuf>,
    extra_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Some(gd) = global_dir {
        if gd.exists() {
            dirs.push(gd);
        }
    }
    if let Some(ad) = agent_skills_dir {
        if ad.exists() {
            dirs.push(ad);
        }
    }
    if let Some(pr) = project_root {
        if pr.exists() {
            dirs.push(pr);
        }
    }
    for ed in extra_dirs {
        if ed.exists() {
            dirs.push(ed);
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
    fn test_skill_dirs_contains_global_only_when_exists() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("home/user/.closeclaw/eda");
        std::fs::create_dir_all(&config_dir).unwrap();

        let global_dir = derive_global_dir(config_dir.to_str().unwrap()).unwrap();
        std::fs::create_dir_all(&global_dir).unwrap();

        let skill_dirs = build_skill_dirs(Some(global_dir.clone()), None, None, vec![]);

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

        // Simulate what init_skill_hot_reload does: extract agent_id from
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

    // --- Step 1.4 tests: hot reload watch dirs include Agent and Project layers ---

    /// build_skill_dirs includes agent_skills_dir when the directory exists on disk.
    #[test]
    fn test_build_skill_dirs_includes_agent_layer_when_exists() {
        let tmp = TempDir::new().unwrap();
        let agent_skills_dir = tmp.path().join("agents/eda/skills");
        std::fs::create_dir_all(&agent_skills_dir).unwrap();

        let skill_dirs = build_skill_dirs(None, Some(agent_skills_dir.clone()), None, vec![]);

        assert_eq!(skill_dirs.len(), 1);
        assert!(skill_dirs.contains(&agent_skills_dir));
    }

    /// build_skill_dirs includes project skills dir when the directory exists on disk.
    /// Note: project_root passed to build_skill_dirs is already the skills dir
    /// (i.e. `<project-root>/.closeclaw/skills/`), not the project root itself.
    #[test]
    fn test_build_skill_dirs_includes_project_layer_when_exists() {
        let tmp = TempDir::new().unwrap();
        let project_skills_dir = tmp.path().join("my/project/.closeclaw/skills");
        std::fs::create_dir_all(&project_skills_dir).unwrap();

        let skill_dirs = build_skill_dirs(None, None, Some(project_skills_dir.clone()), vec![]);

        assert_eq!(skill_dirs.len(), 1);
        assert!(skill_dirs.contains(&project_skills_dir));
    }

    /// build_skill_dirs skips nonexistent directories without panicking.
    #[test]
    fn test_build_skill_dirs_skips_nonexistent_directories() {
        let tmp = TempDir::new().unwrap();
        let agent_skills_dir = tmp.path().join("agents/eda/skills");
        let project_skills_dir = tmp.path().join("my/project/.closeclaw/skills");
        // Intentionally do NOT create these dirs

        let skill_dirs = build_skill_dirs(
            None,
            Some(agent_skills_dir),
            Some(project_skills_dir),
            vec![],
        );

        assert!(skill_dirs.is_empty());
    }

    /// build_skill_dirs includes both global and agent dirs when both exist.
    #[test]
    fn test_build_skill_dirs_includes_global_and_agent_when_both_exist() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("global_skills");
        let agent_skills_dir = tmp.path().join("agents/eda/skills");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::create_dir_all(&agent_skills_dir).unwrap();

        let skill_dirs = build_skill_dirs(
            Some(global_dir.clone()),
            Some(agent_skills_dir.clone()),
            None,
            vec![],
        );

        assert_eq!(skill_dirs.len(), 2);
        assert!(skill_dirs.contains(&global_dir));
        assert!(skill_dirs.contains(&agent_skills_dir));
    }

    /// build_skill_dirs includes all three layers when all dirs exist.
    #[test]
    fn test_build_skill_dirs_includes_all_three_layers() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("global_skills");
        let agent_skills_dir = tmp.path().join("agents/eda/skills");
        let project_skills_dir = tmp.path().join("my/project/.closeclaw/skills");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::create_dir_all(&agent_skills_dir).unwrap();
        std::fs::create_dir_all(&project_skills_dir).unwrap();

        let skill_dirs = build_skill_dirs(
            Some(global_dir.clone()),
            Some(agent_skills_dir.clone()),
            Some(project_skills_dir.clone()),
            vec![],
        );

        assert_eq!(skill_dirs.len(), 3);
        assert!(skill_dirs.contains(&global_dir));
        assert!(skill_dirs.contains(&agent_skills_dir));
        assert!(skill_dirs.contains(&project_skills_dir));
    }

    // --- Step 1.4 tests: ExtraDirs in watcher list ---

    /// build_skill_dirs includes extra_dirs that exist on disk.
    #[test]
    fn test_build_skill_dirs_includes_existing_extra_dirs() {
        let tmp = TempDir::new().unwrap();
        let extra_dir = tmp.path().join("extra/skills");
        std::fs::create_dir_all(&extra_dir).unwrap();

        let skill_dirs = build_skill_dirs(None, None, None, vec![extra_dir.clone()]);

        assert_eq!(skill_dirs.len(), 1);
        assert!(skill_dirs.contains(&extra_dir));
    }

    /// build_skill_dirs skips nonexistent extra_dirs.
    #[test]
    fn test_build_skill_dirs_skips_nonexistent_extra_dirs() {
        let tmp = TempDir::new().unwrap();
        let extra_dir = tmp.path().join("missing/skills");
        // Intentionally do NOT create the dir

        let skill_dirs = build_skill_dirs(None, None, None, vec![extra_dir]);

        assert!(skill_dirs.is_empty());
    }

    /// build_skill_dirs includes extra_dirs alongside other layers.
    #[test]
    fn test_build_skill_dirs_extra_dirs_with_other_layers() {
        let tmp = TempDir::new().unwrap();
        let global_dir = tmp.path().join("global_skills");
        let extra_dir = tmp.path().join("extra/skills");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::create_dir_all(&extra_dir).unwrap();

        let skill_dirs = build_skill_dirs(
            Some(global_dir.clone()),
            None,
            None,
            vec![extra_dir.clone()],
        );

        assert_eq!(skill_dirs.len(), 2);
        assert!(skill_dirs.contains(&global_dir));
        assert!(skill_dirs.contains(&extra_dir));
    }

    /// build_skill_dirs: mix of existing and nonexistent extra_dirs.
    #[test]
    fn test_build_skill_dirs_mixed_extra_dirs() {
        let tmp = TempDir::new().unwrap();
        let existing = tmp.path().join("exists/skills");
        let missing = tmp.path().join("missing/skills");
        std::fs::create_dir_all(&existing).unwrap();

        let skill_dirs = build_skill_dirs(None, None, None, vec![existing.clone(), missing]);

        assert_eq!(skill_dirs.len(), 1);
        assert!(skill_dirs.contains(&existing));
    }

    /// build_skill_dirs: empty extra_dirs has no effect.
    #[test]
    fn test_build_skill_dirs_empty_extra_dirs() {
        let skill_dirs = build_skill_dirs(None, None, None, vec![]);
        assert!(skill_dirs.is_empty());
    }
}
