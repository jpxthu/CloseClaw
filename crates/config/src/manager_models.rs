//! models.json single-point access on [`ConfigManager`].
//!
//! One function domain for everything models.json: the load-time section
//! load that distinguishes the two cases the design docs treat
//! differently — file **absent** → optional (INFO, startup continues:
//! design doc `docs/design/daemon/README.md` 「models.json 缺失…系统仍
//! 正常启动」) vs file **present but corrupt (unparseable)** → F3
//! protection (design doc `docs/design/config/README.md` 启动加载 step 1
//! and `docs/requirements/config.md` §F3: roll back to the latest
//! backup, no usable backup → `Err`, startup refused) — plus the typed
//! parse, whose result / failure state is cached once at load and only
//! read by [`ConfigManager::models_config`] (INFO/WARN logged once at
//! fill time, never per call).

use crate::manager::{ConfigLoadError, ConfigManager, ConfigSection};
use crate::providers::ModelsConfigData;
use std::collections::HashMap;
use std::fs;
use tracing::{info, warn};

/// Cached outcome of the one-shot models.json typed parse.
#[derive(Debug, Clone, Default)]
pub(crate) enum ModelsConfigCache {
    /// models.json absent (or `load()` not run yet) — empty default.
    #[default]
    Absent,
    /// Typed parse succeeded at load.
    Parsed(ModelsConfigData),
    /// models.json present but the typed parse failed (WARN logged once
    /// at fill time) — empty default.
    ParseFailed,
}

impl ConfigManager {
    /// Load the optional Models section and fill the typed cache.
    ///
    /// - File absent → INFO, section stays unset, startup continues
    ///   (design doc daemon README: 「models.json 缺失…系统仍正常启动」).
    /// - File present but unparseable (corrupt) → F3 (design doc config
    ///   README 启动加载 + requirements config §F3): roll back to the
    ///   latest backup and retry — usable backup → WARN + continue,
    ///   no usable backup → [`ConfigLoadError`] (startup refused).
    pub(crate) fn load_models_section(
        &self,
        sections: &mut HashMap<ConfigSection, serde_json::Value>,
    ) -> Result<(), ConfigLoadError> {
        let path = ConfigSection::Models.path(&self.config_dir);
        if !path.exists() {
            info!("{} not found, using defaults", ConfigSection::Models);
            return Ok(());
        }
        let content = fs::read_to_string(&path).map_err(|e| ConfigLoadError::IoError {
            path: path.clone(),
            error: e.to_string(),
        })?;
        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(value) => {
                sections.insert(ConfigSection::Models, value);
                info!(path = %path.display(), "{} loaded", ConfigSection::Models);
            }
            Err(_) => {
                // Corrupt file → F3 rollback path (no usable backup → Err).
                self.try_rollback_and_retry(&path, ConfigSection::Models, sections)?;
            }
        }
        self.cache_models_config(sections.get(&ConfigSection::Models));
        Ok(())
    }

    /// Typed access to models.json — reads the cache filled once at load
    /// (the single parse point, see the module docs).
    ///
    /// Absent or typed-parse failure → [`ModelsConfigData::default`] (no
    /// LLM configured); that outcome was logged once when the cache was
    /// filled, not per call. Consumers treat the default as "no provider
    /// or model configured" (empty LLM fallback chain, startup proceeds).
    pub fn models_config(&self) -> ModelsConfigData {
        self.parsed_models_config().unwrap_or_default()
    }

    /// The cached typed parse — `None` when models.json is absent or its
    /// typed parse failed (both decided at load, logged there).
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

    /// Fill the models cache — the single point for the typed parse and
    /// its logging. Called from [`Self::load_models_section`] (at load)
    /// and [`Self::refresh_models_cache`] (on section writes).
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

#[cfg(test)]
#[path = "manager_models_tests.rs"]
mod tests;
