//! Hot-reload extension for ConfigManager.
//!
//! Provides `reload_section()` for updating a single config section's
//! in-memory cache with validation support.
//!
//! Note: `reload_section` reads from the canonical section file path
//! (`section.path(&self.config_dir)`). It does NOT backup/rollback the
//! file because the file is modified externally (by an editor or tool).
//! Backup/rollback is the responsibility of `ConfigManager::update()`.

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
    /// Flow: read canonical file → parse JSON → validate →
    ///   success: update in-memory cache, emit Reloaded event
    ///   failure: keep old in-memory value, emit Failed event
    ///
    /// This method does NOT backup or rollback the file. The file is
    /// modified externally (editor, CLI, etc.), so ConfigManager has no
    /// authority to revert it. Backup/rollback belongs to `update()`.
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
        let value: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
            self.notify_change(ConfigChangeEvent::Failed {
                section,
                error: e.to_string(),
            });
            ConfigLoadError::ParseError {
                path: path.clone(),
                error: e.to_string(),
            }
        })?;

        // Step 3: validate
        if let Some(validate_fn) = validator {
            if let Err(msg) = validate_fn(&value) {
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
}

#[cfg(test)]
#[path = "manager_reload_tests.rs"]
mod tests;
