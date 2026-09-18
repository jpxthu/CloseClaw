//! models.json single-point access on [`ConfigManager`].
//!
//! One function domain for everything models.json: the load-time section
//! load implementing the four-case matrix the design docs define (no
//! fifth case — config README 启动加载 (startup load) step 1 +
//! 校验规则表 (validation rules table) + daemon README: a missing
//! models.json never blocks startup):
//!
//! 1. File **absent** → optional: INFO, section + cache cleared (the
//!    cache mirrors disk), startup continues.
//! 2. File **present but corrupt** (file-level JSON parse failure) → F3
//!    (design doc `docs/design/config/README.md` 启动加载 (startup
//!    load) step 1 and `docs/requirements/config.md` §F3: roll back to
//!    the latest backup, no usable backup → [`ConfigLoadError`],
//!    startup refused).
//! 3. **Structured parse failure** (valid JSON, wrong shape for
//!    [`ModelsConfigData`]) → F3, same path.
//! 4. **Business validation failure** (non-empty provider/model id,
//!    well-formed base_url — the models row of the validation table) →
//!    F3, same path.
//!
//! Cases 2–4 share [`ConfigManager::rollback_models`] behind the
//! [`gate_models_value`] gate. The cache is written at exactly one place
//! ([`ConfigManager::set_models_cache`]): at load from the gate's typed
//! parse (or the rollback outcome) and on section writes from
//! [`ConfigManager::cache_models_config`]; [`ConfigManager::models_config`]
//! only reads it (logged where filled, never per call).

use crate::manager::{ConfigLoadError, ConfigManager, ConfigSection};
use crate::providers::ModelsConfigData;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{info, warn};

/// Cached outcome of the one-shot models.json typed parse.
#[derive(Debug, Clone, Default)]
pub(crate) enum ModelsConfigCache {
    /// models.json absent (or `load()` not run yet) — empty default.
    #[default]
    Absent,
    /// Typed parse succeeded at load.
    Parsed(ModelsConfigData),
    /// Typed parse failed on a section **write** (WARN logged once when
    /// the cache was filled) — empty default. Load-time parse failures
    /// are refused via F3 and never reach the cache.
    ParseFailed,
}

// ---------------------------------------------------------------------------
// Load path — four-case matrix + F3 rollback
// ---------------------------------------------------------------------------

impl ConfigManager {
    /// Load the optional Models section and fill the typed cache,
    /// implementing the four-case matrix above.
    ///
    /// - File absent → INFO, section + cache cleared, startup continues
    ///   (design doc daemon README: the system still starts when
    ///   models.json is missing).
    /// - File present → parse / business-validation / structured-parse
    ///   failures each go through F3 ([`Self::rollback_models`]: roll
    ///   back to the latest backup; usable backup → continue with the
    ///   backup's value, no usable backup → [`ConfigLoadError`],
    ///   startup refused).
    pub(crate) fn load_models_section(
        &self,
        sections: &mut HashMap<ConfigSection, serde_json::Value>,
    ) -> Result<(), ConfigLoadError> {
        let path = ConfigSection::Models.path(&self.config_dir);
        if !path.exists() {
            // The cache mirrors disk: clear any value from a previous
            // load so `models_config()` cannot read a stale entry.
            sections.remove(&ConfigSection::Models);
            self.cache_models_config(None);
            info!("{} not found, using defaults", ConfigSection::Models);
            return Ok(());
        }
        let content = fs::read_to_string(&path).map_err(|e| ConfigLoadError::IoError {
            path: path.clone(),
            error: e.to_string(),
        })?;
        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(err) => {
                return self.rollback_models(&path, sections, format!("file parse failed: {err}"));
            }
        };
        match gate_models_value(&value) {
            Ok(data) => {
                sections.insert(ConfigSection::Models, value);
                info!(path = %path.display(), "{} loaded", ConfigSection::Models);
                self.set_models_cache(ModelsConfigCache::Parsed(data));
            }
            Err(reason) => self.rollback_models(&path, sections, reason)?,
        }
        Ok(())
    }

    /// One shared F3 path for cases 2–4: log the rejection reason, roll
    /// back to the latest backup (usable backup → `try_rollback_and_retry`
    /// inserts its value and returns `Ok`, no usable backup → `Err`,
    /// startup refused), then refill the cache from the outcome.
    fn rollback_models(
        &self,
        path: &Path,
        sections: &mut HashMap<ConfigSection, serde_json::Value>,
        reason: String,
    ) -> Result<(), ConfigLoadError> {
        warn!(
            %reason,
            path = %path.display(),
            "models.json rejected — attempting rollback to backup"
        );
        self.try_rollback_and_retry(path, ConfigSection::Models, sections)?;
        self.cache_models_config(sections.get(&ConfigSection::Models));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Typed access + cache write points
// ---------------------------------------------------------------------------

impl ConfigManager {
    /// Typed access to models.json — reads the cache filled once at load
    /// (the single cache-write point, see the module docs).
    ///
    /// Absent file (or a section write whose typed parse failed) →
    /// [`ModelsConfigData::default`] (no LLM configured); that outcome
    /// was logged when the cache was filled, not per call. Consumers
    /// treat the default as "no provider or model configured" (empty LLM
    /// fallback chain, startup proceeds). Load-time failures of a present
    /// file are refused earlier by [`Self::load_models_section`] (F3).
    pub fn models_config(&self) -> ModelsConfigData {
        self.parsed_models_config().unwrap_or_default()
    }

    /// The cached typed parse — `None` when models.json is absent or its
    /// typed parse failed on a section write (both decided where the
    /// cache was filled, logged there).
    pub(crate) fn parsed_models_config(&self) -> Option<ModelsConfigData> {
        match &*self
            .models_cache
            .read()
            .expect("RwLock for models cache was poisoned")
        {
            ModelsConfigCache::Parsed(data) => Some(data.clone()),
            ModelsConfigCache::Absent | ModelsConfigCache::ParseFailed => None,
        }
    }

    /// Fill the models cache from a raw value — the typed parse and its
    /// logging for section writes and rollback outcomes (load success
    /// stores the gate's parse result directly via [`Self::set_models_cache`]).
    /// Called from [`Self::load_models_section`] / [`Self::rollback_models`]
    /// (at load) and [`Self::refresh_models_cache`] (on section writes).
    fn cache_models_config(&self, value: Option<&serde_json::Value>) {
        let cache = match value {
            None => ModelsConfigCache::Absent,
            Some(value) => match serde_json::from_value::<ModelsConfigData>(value.clone()) {
                Ok(data) => ModelsConfigCache::Parsed(data),
                Err(err) => {
                    warn!(%err, "models.json typed parse failed — using empty model config");
                    ModelsConfigCache::ParseFailed
                }
            },
        };
        self.set_models_cache(cache);
    }

    /// The one place the cache lock is taken for writing.
    fn set_models_cache(&self, cache: ModelsConfigCache) {
        *self
            .models_cache
            .write()
            .expect("RwLock for models cache was poisoned") = cache;
    }

    /// Refresh the models cache when the Models section is written
    /// (`update` / `update_section_cache`) so `models_config()` never
    /// reads a stale value. Other sections are ignored.
    pub(crate) fn refresh_models_cache(&self, section: ConfigSection, value: &serde_json::Value) {
        if section == ConfigSection::Models {
            self.cache_models_config(Some(value));
        }
    }
}

/// Gate a parsed models.json value before it is accepted into memory:
/// business validation (non-empty provider/model id, well-formed
/// base_url), then the structured parse into [`ModelsConfigData`].
/// `Err(reason)` sends the caller down the F3 rollback path.
fn gate_models_value(value: &serde_json::Value) -> Result<ModelsConfigData, String> {
    crate::validators::for_section(ConfigSection::Models)(value)
        .map_err(|err| format!("business validation failed: {err}"))?;
    serde_json::from_value::<ModelsConfigData>(value.clone())
        .map_err(|err| format!("structured parse failed: {err}"))
}

#[cfg(test)]
#[path = "manager_models_tests.rs"]
mod tests;
