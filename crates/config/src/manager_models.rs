//! models.json single-point access on [`ConfigManager`].
//!
//! models.json is an **optional** section at load: a missing file never
//! blocks startup (design doc `docs/design/daemon/README.md`:
//! 「models.json 缺失…系统仍正常启动」) and an unparseable file logs a
//! WARN with the empty default. Every typed consumer — the daemon LLM
//! registry init, the credential_path merge, and the cross-file
//! validation — goes through this single parse point so INFO/WARN
//! logging and default semantics live in exactly one place.

use crate::manager::{ConfigManager, ConfigSection};
use crate::providers::ModelsConfigData;
use tracing::{info, warn};

impl ConfigManager {
    /// Typed access to models.json — the single parse point for the
    /// Models section.
    ///
    /// Missing section → INFO + [`ModelsConfigData::default`] (no
    /// LLM configured); present but unparseable → WARN + default.
    /// Never fails — consumers treat the default as "no provider or
    /// model configured" (empty LLM fallback chain, startup proceeds).
    pub fn models_config(&self) -> ModelsConfigData {
        match self.section(ConfigSection::Models) {
            Some(value) => parse_models_config(&value),
            None => {
                info!("models.json not configured — using empty model config");
                ModelsConfigData::default()
            }
        }
    }
}

/// Parse a present models.json section value into [`ModelsConfigData`].
///
/// Typed parse failure → WARN + empty default (non-blocking — the
/// Models section is loaded as an optional section, see
/// [`ConfigManager::load`]).
pub(crate) fn parse_models_config(value: &serde_json::Value) -> ModelsConfigData {
    serde_json::from_value::<ModelsConfigData>(value.clone()).unwrap_or_else(|err| {
        warn!(%err, "models.json parse failed — using empty model config");
        ModelsConfigData::default()
    })
}

#[cfg(test)]
#[path = "manager_models_tests.rs"]
mod tests;
