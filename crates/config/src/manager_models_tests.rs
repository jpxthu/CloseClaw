//! Tests for the models.json single-point access (`models_config`) and
//! the optional-section load semantics of models.json.

use super::*;
use std::fs;

/// The 5 mandatory config files — models.json deliberately absent.
fn write_mandatory_without_models(dir: &std::path::Path) {
    for name in [
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        fs::write(
            dir.join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )
        .unwrap();
    }
}

/// models.json absent → load() succeeds (optional section), the section
/// stays `None`, and `models_config()` returns the empty default.
/// Design doc daemon README: 「models.json 缺失…系统仍正常启动」.
#[test]
fn models_config_missing_returns_default() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    manager
        .load()
        .expect("missing models.json must not fail load");
    assert!(
        manager.section(ConfigSection::Models).is_none(),
        "absent file → section stays None"
    );

    let models = manager.models_config();
    assert!(models.providers.is_empty(), "default = no providers");
    assert_eq!(models.mode, "merge");
}

/// models.json unparseable (invalid JSON) → load() succeeds (WARN,
/// optional section), the section stays `None`, `models_config()`
/// returns the empty default — never a hard error.
#[test]
fn models_config_corrupt_file_returns_default() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path());
    fs::write(tmp.path().join("models.json"), "not valid json {{").unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    manager
        .load()
        .expect("corrupt models.json must not fail load");
    assert!(manager.section(ConfigSection::Models).is_none());
    assert!(manager.models_config().providers.is_empty());
}

/// models.json parses as JSON but not as [`ModelsConfigData`] →
/// `models_config()` takes the WARN branch and returns the empty
/// default while the raw section value stays readable.
#[test]
fn models_config_untyped_value_returns_default() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path());
    fs::write(tmp.path().join("models.json"), r#"{"providers":"nope"}"#).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    manager
        .load()
        .expect("load must not fail on untyped models.json");
    assert!(
        manager.section(ConfigSection::Models).is_some(),
        "raw JSON value is still cached"
    );
    assert!(manager.models_config().providers.is_empty());
}
