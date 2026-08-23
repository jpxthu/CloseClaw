//! Skill-related helper utilities.

use closeclaw_config::{ConfigManager, ConfigSection};
use std::path::PathBuf;

/// Resolve extra skill directories from skills.json.
///
/// Reads `extraDirs` from `SkillsConfigData`, expands `~` to home.
/// Non-existent paths are kept as-is (loader skips them).
pub(crate) fn resolve_extra_dirs(config_manager: &ConfigManager) -> Vec<PathBuf> {
    let Some(v) = config_manager.section(ConfigSection::Skills) else {
        return Vec::new();
    };
    let Ok(cfg) = serde_json::from_value::<closeclaw_config::SkillsConfigData>(v) else {
        return Vec::new();
    };
    let home = dirs::home_dir();
    cfg.config
        .extra_dirs
        .iter()
        .map(|d| {
            if let Some(r) = d.strip_prefix("~/") {
                home.as_ref()
                    .map(|h| h.join(r))
                    .unwrap_or_else(|| PathBuf::from(d))
            } else {
                PathBuf::from(d)
            }
        })
        .collect()
}
