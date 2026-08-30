//! Pending-restart staging methods for restart-class config sections.
//!
//! Extracted from `manager.rs` to keep file sizes under limits.
//! These methods manage a staging area for restart-class config values
//! (Models, Channels, Gateway) that are only committed to runtime cache
//! after a gateway restart completes.

use std::path::PathBuf;

use tracing::info;

use crate::events::ConfigChangeEvent;

use super::ConfigManager;
use super::ConfigSection;

impl ConfigManager {
    /// Stage a validated value for a restart-class config section.
    ///
    /// Writes to the pending-restart area without updating the in-memory
    /// cache. Emits `ConfigChangeEvent::Reloaded` so downstream consumers
    /// can react (e.g. trigger a restart).
    pub fn stage_restart_value(
        &self,
        section: ConfigSection,
        path: PathBuf,
        value: serde_json::Value,
    ) {
        self.pending_restart
            .write()
            .expect("RwLock for pending_restart was poisoned")
            .insert(section, value);
        self.notify_change(ConfigChangeEvent::Reloaded { section, path });
        info!(section = %section, "staged restart-class config value");
    }

    /// Apply all staged restart-class values to the runtime cache.
    ///
    /// Called after a gateway restart completes. Moves each staged value
    /// into the in-memory cache via `update_section_cache()` and clears
    /// the pending-restart map.
    pub fn apply_pending_restart(&self) {
        let staged: Vec<(ConfigSection, serde_json::Value)> = {
            self.pending_restart
                .write()
                .expect("RwLock for pending_restart was poisoned")
                .drain()
                .collect()
        };
        for (section, value) in staged {
            let path = section.path(&self.config_dir);
            self.update_section_cache(section, path, value);
        }
    }

    /// Query a staged restart-class value without consuming it.
    pub fn pending_restart_value(&self, section: ConfigSection) -> Option<serde_json::Value> {
        self.pending_restart
            .read()
            .expect("RwLock for pending_restart was poisoned")
            .get(&section)
            .cloned()
    }
}
