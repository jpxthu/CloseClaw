//! Tests for the models.json single-point access (`models_config`) and
//! the optional-section load semantics of models.json (missing → INFO +
//! default; corrupt without backup → F3 refusal).

use super::*;
use closeclaw_common::test_helpers::write_mandatory_without_models;
use std::fs;

/// models.json absent → load() succeeds (optional section), the section
/// stays `None`, and `models_config()` returns the empty default.
/// Design doc daemon README: 「models.json 缺失…系统仍正常启动」.
#[test]
fn models_config_missing_returns_default() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
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

/// models.json present but corrupt (invalid JSON) and no backup exists
/// → F3 protection (config README 启动加载 step 1 + requirements
/// config §F3): `load()` refuses startup. The typed-parse failure state
/// cached for the accessor covers valid-JSON-wrong-shape only (next
/// test) — file-level corruption never reaches the accessor.
#[test]
fn models_config_corrupt_file_without_backup_refuses_load() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
    fs::write(tmp.path().join("models.json"), "not valid json {{").unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    let err = manager
        .load()
        .expect_err("corrupt models.json without backup must refuse load");
    assert!(err.to_string().contains("models.json"), "{err}");
}

/// models.json parses as JSON but not as [`ModelsConfigData`] →
/// `load()` warns once while filling the cache; `models_config()`
/// returns the empty default while the raw section value stays
/// readable.
#[test]
fn models_config_untyped_value_returns_default() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
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
