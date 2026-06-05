//! Hot-reload extension for ConfigManager.
//!
//! Provides `reload_section()` for updating a single config section's
//! in-memory cache from validated file content without backup or disk write.

use super::manager::{ConfigLoadError, ConfigManager, ConfigSection};
use tracing::info;

impl ConfigManager {
    /// Hot-reload a single section from file content (no backup/write).
    pub fn reload_section(
        &self,
        section: ConfigSection,
        content: &str,
    ) -> Result<(), ConfigLoadError> {
        let value: serde_json::Value =
            serde_json::from_str(content).map_err(|e| ConfigLoadError::ParseError {
                path: section.path(&self.config_dir),
                error: e.to_string(),
            })?;
        let mut sections = self
            .sections
            .write()
            .expect("RwLock for config sections was poisoned");
        sections.insert(section, value);
        info!("reloaded config section: {}", section);
        Ok(())
    }
}

#[cfg(test)]
#[path = "manager_reload_tests.rs"]
mod tests;
