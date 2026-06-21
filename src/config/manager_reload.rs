//! Hot-reload extension for ConfigManager.
//!
//! Provides `reload_section()` for updating a single config section's
//! in-memory cache with backup, validation, and rollback support.

use super::manager::{ConfigLoadError, ConfigManager, ConfigSection};
use tracing::{info, warn};

/// Validator callback type for config section reload.
///
/// Receives the parsed JSON value and returns `Ok(())` if valid,
/// or `Err(message)` to reject the reload.
pub type SectionValidator = dyn Fn(&serde_json::Value) -> Result<(), String>;

impl ConfigManager {
    /// Hot-reload a single section from file content.
    ///
    /// Flow: backup current file → parse JSON → validate →
    ///   success: update in-memory cache
    ///   failure: rollback file, keep old in-memory value, log error
    ///
    /// The `validator` callback performs additional business validation
    /// on the parsed JSON value. Return `Ok(())` to accept, or
    /// `Err(message)` to reject.
    pub fn reload_section(
        &self,
        section: ConfigSection,
        content: &str,
        validator: Option<&SectionValidator>,
    ) -> Result<(), ConfigLoadError> {
        let path = section.path(&self.config_dir);

        // Step 1: backup current file before overwriting
        if path.exists() {
            if let Err(e) = self.backup_manager.backup(&path) {
                warn!(
                    error = %e, section = %section,
                    "failed to backup config before reload"
                );
            }
        }

        // Step 2: parse new content
        let value: serde_json::Value = serde_json::from_str(content).map_err(|e| {
            // Parse failed → rollback file
            if let Err(rb) = self.backup_manager.rollback(&path) {
                warn!(
                    error = %rb, section = %section,
                    "rollback also failed after parse error"
                );
            }
            ConfigLoadError::ParseError {
                path: path.clone(),
                error: e.to_string(),
            }
        })?;

        // Step 3: validate
        if let Some(validate_fn) = validator {
            if let Err(msg) = validate_fn(&value) {
                // Validation failed → rollback file
                if let Err(rb) = self.backup_manager.rollback(&path) {
                    warn!(
                        error = %rb, section = %section,
                        "rollback also failed after validation error"
                    );
                }
                return Err(ConfigLoadError::ValidationError { path, message: msg });
            }
        }

        // Step 4: success — update in-memory cache
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
