//! Hot-reload extension for ConfigManager.
//!
//! Provides `reload_section()` for updating a single config section's
//! in-memory cache with validation support.
//!
//! Note: `reload_section` reads from the canonical section file path
//! (`section.path(&self.config_dir)`). On parse/validation failure, the
//! file is NOT rolled back (per design doc). If the section had no
//! previous in-memory value (first load), it is blocked so that
//! `get_section_value` returns `None`.

use std::sync::Arc;

use crate::events::ConfigChangeEvent;
use crate::manager::{ConfigLoadError, ConfigManager, ConfigSection};
use crate::session::{JsonSessionConfigProvider, SessionConfigProvider};
use crate::SectionValidator;
use tracing::warn;

impl ConfigManager {
    /// Hot-reload a single section by reading its canonical file.
    ///
    /// Flow: read file → parse JSON → validate →
    ///   success: update in-memory cache, emit Reloaded event
    ///   failure: keep old in-memory value, do NOT roll back file per design doc,
    ///            emit Failed event. If no old value existed, block the section.
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
                    path: path.clone(),
                    error: e.to_string(),
                });
                return Err(ConfigLoadError::IoError {
                    path,
                    error: e.to_string(),
                });
            }
        };

        // Step 2: backup the old in-memory value before replacing it
        let old_value = self
            .sections
            .read()
            .expect("RwLock for config sections was poisoned")
            .get(&section)
            .cloned();
        if let Some(ref old) = old_value {
            let old_json = serde_json::to_string(old).unwrap_or_default();
            if let Err(e) = self
                .backup_manager
                .backup_with_content(&path, old_json.as_bytes())
            {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to backup config content before reload"
                );
            }
        }

        // Step 3: parse new content
        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                if old_value.is_none() {
                    self.block_section(section);
                }
                self.notify_change(ConfigChangeEvent::Failed {
                    section,
                    path: path.clone(),
                    error: e.to_string(),
                });
                return Err(ConfigLoadError::ParseError {
                    path,
                    error: e.to_string(),
                });
            }
        };

        // Step 4: validate
        if let Some(validate_fn) = validator {
            if let Err(msg) = validate_fn(&value) {
                if old_value.is_none() {
                    self.block_section(section);
                }
                self.notify_change(ConfigChangeEvent::Failed {
                    section,
                    path: path.clone(),
                    error: msg.clone(),
                });
                return Err(ConfigLoadError::ValidationError { path, message: msg });
            }
        }

        // Step 5: success — update in-memory cache and broadcast snapshot
        self.update_section_cache(section, path, value);
        Ok(())
    }

    /// Rebuild the session config provider from the current session.json content.
    ///
    /// Called after session.json is hot-reloaded to keep the typed provider
    /// (used by ArchiveSweeper) in sync with the raw JSON in `sections`.
    pub fn reload_session_provider(&self) {
        let path = ConfigSection::Session.path(&self.config_dir);
        let provider: Arc<dyn SessionConfigProvider> = match JsonSessionConfigProvider::new(&path) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to rebuild session provider, using defaults"
                );
                Arc::new(JsonSessionConfigProvider::default())
            }
        };
        *self.session_provider.write().expect("RwLock poisoned") = Some(provider);
    }
}

#[cfg(test)]
#[path = "manager_reload_tests.rs"]
mod tests;
