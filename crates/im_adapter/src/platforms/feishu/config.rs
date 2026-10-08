//! Platform configuration loading for the Feishu plugin.
//!
//! Loads `platforms.json` (per-platform enablement) from the config
//! directory.

use serde::Deserialize;
use std::collections::HashMap;
use tracing::{info, warn};

/// Root platforms configuration loaded from `platforms.json`.
///
/// Each key is a platform name and `enabled` controls whether
/// the platform plugin is registered at startup.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct PlatformsConfig {
    platforms: HashMap<String, PlatformEnabledEntry>,
}

/// A single platform entry in `platforms.json`.
#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct PlatformEnabledEntry {
    #[serde(default)]
    enabled: bool,
}

impl PlatformsConfig {
    /// Check whether a platform is explicitly enabled.
    pub(super) fn is_enabled(&self, platform: &str) -> bool {
        self.platforms.get(platform).is_some_and(|e| e.enabled)
    }
}

/// Load `{config_dir}/config/platforms.json`.
///
/// Returns an empty config when the file is missing or unparseable.
pub(crate) fn load_platforms_config(config_dir: &str) -> PlatformsConfig {
    let path = std::path::Path::new(config_dir)
        .join("config")
        .join("platforms.json");
    match std::fs::read_to_string(&path) {
        Ok(json) => match serde_json::from_str::<PlatformsConfig>(&json) {
            Ok(cfg) => cfg,
            Err(e) => {
                warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to parse platforms.json — all platforms disabled"
                );
                PlatformsConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("platforms.json not found — all platforms disabled");
            PlatformsConfig::default()
        }
        Err(e) => {
            warn!(
                error = %e,
                path = %path.display(),
                "failed to read platforms.json — all platforms disabled"
            );
            PlatformsConfig::default()
        }
    }
}
