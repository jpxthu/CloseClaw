//! Skill Hot Reload Initialization
//!
//! Initializes the skill registry and file watcher at daemon startup.

use crate::skills::{
    init_disk_skills, start_skill_watcher, DiskSkillRegistry, ScanConfig, SkillWatcherHandle,
};
use crate::system_prompt::sections::invalidate_skill_listing;
use std::path::PathBuf;
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
) -> anyhow::Result<(Arc<RwLock<Option<DiskSkillRegistry>>>, SkillWatcherHandle)> {
    let skill_dir = std::path::Path::new(config_dir).join("skills");
    let skill_dirs: Vec<PathBuf> = vec![skill_dir.clone()];

    let scan_config = ScanConfig {
        bundled_dir: Some(skill_dir),
        ..Default::default()
    };

    // Initialize shared registry state
    let registry = init_disk_skills(&scan_config);
    let registry_len = registry.len();
    let registry_arc = Arc::new(RwLock::new(Some(registry)));
    let registry_for_watcher = Arc::clone(&registry_arc);

    info!(loaded = registry_len, "skill registry initialized");

    // Start watcher — re-scan uses the same ScanConfig as initial scan
    let watcher_config = scan_config.clone();
    let watcher = start_skill_watcher(
        skill_dirs,
        Box::new(move || {
            let new_registry = init_disk_skills(&watcher_config);

            // Update shared state
            if let Ok(mut guard) = registry_for_watcher.write() {
                *guard = Some(new_registry);
            }

            // Invalidate cache so next build picks up new listing
            invalidate_skill_listing();

            tracing::info!("skill registry reloaded after file change");
        }),
    )?;

    info!("skill hot reload initialized");
    Ok((registry_arc, watcher))
}
