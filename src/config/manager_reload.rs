//! Hot-reload extension for ConfigManager.
//!
//! Provides `reload_section()` for updating a single config section's
//! in-memory cache with validation support.
//!
//! Note: `reload_section` reads from the canonical section file path
//! (`section.path(&self.config_dir)`). On parse/validation failure, the
//! file is restored to the last known good state (from in-memory cache)
//! to keep the on-disk state consistent.

use super::events::ConfigChangeEvent;
use super::manager::{ConfigLoadError, ConfigManager, ConfigSection};
use tracing::info;

/// Validator callback type for config section reload.
///
/// Receives the parsed JSON value and returns `Ok(())` if valid,
/// or `Err(message)` to reject the reload.
pub type SectionValidator = dyn Fn(&serde_json::Value) -> Result<(), String>;

impl ConfigManager {
    /// Hot-reload a single section by reading its canonical file.
    ///
    /// Flow: read file → parse JSON → validate →
    ///   success: update in-memory cache, emit Reloaded event
    ///   failure: keep old in-memory value, restore file to last known good state,
    ///            emit Failed event
    ///
    /// The `validator` callback performs additional business validation
    /// on the parsed JSON value. Return `Ok(())` to accept, or
    /// `Err(message)` to reject.
    pub fn reload_section(
        &self,
        section: ConfigSection,
        validator: Option<&SectionValidator>,
    ) -> Result<(), ConfigLoadError> {
        let path = section.path(&self.config_dir);

        // Step 1: read the canonical config file
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                self.notify_change(ConfigChangeEvent::Failed {
                    section,
                    error: e.to_string(),
                });
                return Err(ConfigLoadError::IoError {
                    path,
                    error: e.to_string(),
                });
            }
        };

        // Step 2: parse new content
        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                self.restore_file_to_last_known_good(&path, section);
                self.notify_change(ConfigChangeEvent::Failed {
                    section,
                    error: e.to_string(),
                });
                return Err(ConfigLoadError::ParseError {
                    path,
                    error: e.to_string(),
                });
            }
        };

        // Step 3: validate
        if let Some(validate_fn) = validator {
            if let Err(msg) = validate_fn(&value) {
                self.restore_file_to_last_known_good(&path, section);
                self.notify_change(ConfigChangeEvent::Failed {
                    section,
                    error: msg.clone(),
                });
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
        self.notify_change(ConfigChangeEvent::Reloaded { section });
        Ok(())
    }

    /// Restore a config file to the last known good state from the in-memory cache.
    ///
    /// Serializes the old value while holding the read lock, then releases
    /// the lock before performing filesystem I/O, avoiding holding the lock
    /// during a potentially slow write operation.
    ///
    /// Logs a warning on failure but does not propagate errors.
    fn restore_file_to_last_known_good(&self, path: &std::path::Path, section: ConfigSection) {
        let content = {
            let sections = self
                .sections
                .read()
                .expect("RwLock for config sections was poisoned");
            match sections.get(&section) {
                Some(old_value) => match serde_json::to_string_pretty(old_value) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("failed to serialize old config for {}: {}", section, e);
                        return;
                    }
                },
                None => return,
            }
        };
        if let Err(e) = std::fs::write(path, content) {
            tracing::warn!("failed to restore config file {}: {}", path.display(), e);
        }
    }
}

#[cfg(test)]
#[path = "manager_reload_tests.rs"]
mod tests;
