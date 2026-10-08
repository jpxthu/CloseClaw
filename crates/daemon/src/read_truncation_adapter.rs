//! Adapter: build the Read tool's truncation provider from ConfigManager.
//!
//! The `tools.json` parsing (`read.max_tokens`) lives in the composition
//! root — `closeclaw_tools` only consumes a
//! [`ReadTruncationProvider`](closeclaw_tools::builtin::ReadTruncationProvider).
//! The provider re-reads the Tools section on every invocation so config
//! hot-reloads take effect without a restart.

use closeclaw_config::{ConfigManager, ConfigSection};
use closeclaw_tools::builtin::{ReadTruncationProvider, TruncationConfig};
use std::sync::Arc;

/// Default maximum number of lines returned per Read call.
#[cfg(test)]
const DEFAULT_MAX_LINES: usize = 2000;

/// Default maximum byte size (50 KB) returned per Read call.
#[cfg(test)]
const DEFAULT_MAX_BYTES: usize = 51_200;

/// Build a [`ReadTruncationProvider`] backed by `ConfigManager`.
///
/// The returned closure re-resolves the truncation config on every
/// call, preserving the pre-existing hot-reload semantics of
/// `TruncationConfig::from_config`.
pub fn read_truncation_provider(cm: Arc<ConfigManager>) -> ReadTruncationProvider {
    Arc::new(move || truncation_config(&cm))
}

/// Resolve the truncation config from the `tools.json` section.
///
/// Reads `read.max_tokens` if present; falls back to
/// [`TruncationConfig::default()`] when the section is absent, the
/// field is missing, or the value is invalid (zero, negative, or
/// non-numeric).
fn truncation_config(cm: &ConfigManager) -> TruncationConfig {
    let mut cfg = TruncationConfig::default();
    if let Some(tools_value) = cm.get_section_value(ConfigSection::Tools) {
        if let Some(read_obj) = tools_value.get("read") {
            if let Some(max_tokens) = read_obj.get("max_tokens") {
                if let Some(v) = max_tokens.as_u64() {
                    if v > 0 {
                        cfg.max_tokens = v as usize;
                    }
                }
            }
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a ConfigManager with an optional tools.json content.
    fn make_config_manager(tools_json: Option<&str>) -> (TempDir, ConfigManager) {
        let tmp = TempDir::new().unwrap();
        // Write tools.json if provided
        if let Some(content) = tools_json {
            std::fs::write(tmp.path().join("tools.json"), content).unwrap();
        }
        // ConfigManager::new + load expects mandatory sections; we bypass
        // load() by directly inserting into the in-memory cache.
        let cm = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
        if let Some(content) = tools_json {
            let value: serde_json::Value = serde_json::from_str(content).unwrap();
            cm.update_section_cache(ConfigSection::Tools, tmp.path().join("tools.json"), value);
        }
        (tmp, cm)
    }

    #[test]
    fn test_truncation_config_valid_max_tokens() {
        let (_tmp, cm) = make_config_manager(Some(r#"{"read": {"max_tokens": 5000}}"#));
        let cfg = truncation_config(&cm);
        assert_eq!(cfg.max_tokens, 5000);
        // Other values remain default
        assert_eq!(cfg.max_lines, DEFAULT_MAX_LINES);
        assert_eq!(cfg.max_bytes, DEFAULT_MAX_BYTES);
    }

    #[test]
    fn test_truncation_config_missing_tools_json() {
        let (_tmp, cm) = make_config_manager(None);
        let cfg = truncation_config(&cm);
        // Falls back to default
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_BYTES / 4);
    }

    #[test]
    fn test_truncation_config_invalid_zero_tokens() {
        let (_tmp, cm) = make_config_manager(Some(r#"{"read": {"max_tokens": 0}}"#));
        let cfg = truncation_config(&cm);
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_BYTES / 4);
    }

    #[test]
    fn test_truncation_config_invalid_negative_tokens() {
        let (_tmp, cm) = make_config_manager(Some(r#"{"read": {"max_tokens": -1}}"#));
        let cfg = truncation_config(&cm);
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_BYTES / 4);
    }

    #[test]
    fn test_truncation_config_invalid_non_number_tokens() {
        let (_tmp, cm) = make_config_manager(Some(r#"{"read": {"max_tokens": "abc"}}"#));
        let cfg = truncation_config(&cm);
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_BYTES / 4);
    }

    #[test]
    fn test_truncation_config_missing_read_section() {
        let (_tmp, cm) = make_config_manager(Some(r#"{}"#));
        let cfg = truncation_config(&cm);
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_BYTES / 4);
    }

    /// The provider re-reads the section on every invocation — a
    /// hot-reload of `read.max_tokens` must be visible on the next call.
    #[test]
    fn test_provider_reflects_hot_reload() {
        let (tmp, cm) = make_config_manager(Some(r#"{"read": {"max_tokens": 5000}}"#));
        let cm = Arc::new(cm);
        let provider = read_truncation_provider(Arc::clone(&cm));
        assert_eq!(provider().max_tokens, 5000);

        // Simulate a hot-reload: update the in-memory section cache.
        let value: serde_json::Value =
            serde_json::from_str(r#"{"read": {"max_tokens": 9000}}"#).unwrap();
        cm.update_section_cache(ConfigSection::Tools, tmp.path().join("tools.json"), value);
        assert_eq!(provider().max_tokens, 9000);
    }
}
