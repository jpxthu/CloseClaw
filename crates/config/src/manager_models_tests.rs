//! Tests for the models.json single-point access (`models_config`) and
//! the four-case load semantics of models.json (missing → INFO +
//! defaults + cache/section cleared; file corruption / structured parse
//! failure / business validation failure → F3 refusal without backup).

use super::*;
use closeclaw_common::test_helpers::{write_mandatory_configs, write_mandatory_without_models};
use std::fs;

/// models.json absent → load() succeeds (optional section), the section
/// stays `None`, and `models_config()` returns the empty default.
/// Design doc daemon README: 「models.json 缺失…系统仍正常启动」.
#[test]
fn test_models_config_missing_returns_default() {
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
/// → F3 protection (config README 启动加载 (startup load) step 1 +
/// requirements config §F3): `load()` refuses startup (case 2 of the
/// matrix; cases
/// 3–4 below take the same rollback path).
#[test]
fn test_models_config_corrupt_file_without_backup_refuses_load() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
    fs::write(tmp.path().join("models.json"), "not valid json {{").unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    let err = manager
        .load()
        .expect_err("corrupt models.json without backup must refuse load");
    assert!(err.to_string().contains("models.json"), "{err}");
}

/// models.json parses as JSON but not as [`ModelsConfigData`] (wrong
/// shape for the typed structure) and no backup exists → F3: `load()`
/// refuses startup (Step 1.20 — the pre-matrix assertion that let this
/// load with only a WARN is reverted).
#[test]
fn test_models_config_untyped_value_refuses_load() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
    fs::write(tmp.path().join("models.json"), r#"{"providers":"nope"}"#).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    let err = manager
        .load()
        .expect_err("structured parse failure without backup must refuse load");
    assert!(err.to_string().contains("models.json"), "{err}");
    assert!(
        manager.models_config().providers.is_empty(),
        "no value may be cached from a refused load"
    );
}

/// Test: models.json business validation failure, no backup → refusal (F3).
/// Moved from `manager_tests.rs` (Step 1.25) to sit with the corrupt /
/// untyped refusal siblings and free up room in the 1000-line file.
#[test]
fn test_load_business_validation_failure_models_refuses_without_backup() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"":{"models":[]}}}"#,
    )
    .unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    let err = manager
        .load()
        .expect_err("business validation failure without backup must refuse load");
    assert!(err.to_string().contains("models.json"), "{err}");
}

/// The cache mirrors disk: reloading after models.json was deleted
/// clears both the raw section and the typed cache — no stale value
/// from the previous load survives (Step 1.20 missing-branch cleanup).
#[test]
fn test_models_config_removed_file_clears_stale_cache() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
    fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai":{"models":[{"id":"m1"}]}}}"#,
    )
    .unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    manager.load().expect("load with models.json");
    assert!(
        manager.models_config().providers.contains_key("openai"),
        "loaded value must be cached"
    );

    fs::remove_file(tmp.path().join("models.json")).unwrap();
    manager
        .load()
        .expect("reload after file removal must not fail");
    assert!(
        manager.section(ConfigSection::Models).is_none(),
        "section must be cleared when the file is gone"
    );
    assert!(
        manager.models_config().providers.is_empty(),
        "cache must be cleared when the file is gone"
    );
}

/// Writing the Models section (`update`) refreshes the cached typed
/// parse: `models_config()` returns the new value, not the load-time
/// one — locks the `refresh_models_cache` write-path wiring.
#[test]
fn test_models_config_update_returns_new_value() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
    fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai":{"models":[{"id":"m1"}]}}}"#,
    )
    .unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().expect("initial load");
    assert!(
        manager.models_config().providers.contains_key("openai"),
        "load-time value must be cached"
    );

    let new_value = serde_json::json!({
        "providers": {
            "anthropic": {"baseUrl": "https://api.anthropic.com", "models": [{"id": "m2"}]}
        }
    });
    manager
        .update(ConfigSection::Models, new_value, |_| Ok(()))
        .expect("update must succeed");

    let models = manager.models_config();
    assert!(
        !models.providers.contains_key("openai"),
        "stale load-time value must be replaced"
    );
    assert!(
        models.providers.contains_key("anthropic"),
        "newly written value must be cached"
    );
}

/// Section write whose value passes the business validator
/// (`validate_models_with_refs` runs inside `update` for the Models
/// section) but fails the typed parse → cache stores `ParseFailed`: the
/// raw value stays in the section while `models_config()` returns the
/// empty default (Step 1.23 — `ModelsConfigCache::ParseFailed` branch).
#[test]
fn test_models_config_write_typed_parse_failure_keeps_empty_default() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().expect("load");

    // `mode` must be a string in `ModelsConfigData`; the business
    // validator only checks the `providers` structure / ids / urls, so
    // this value clears validation and fails the typed parse.
    let value = serde_json::json!({"version": "1.0", "mode": 123});
    manager
        .update(ConfigSection::Models, value.clone(), |_| Ok(()))
        .expect("update must succeed — the value passes business validation");

    assert_eq!(
        manager.section(ConfigSection::Models),
        Some(value),
        "the raw value must be kept in the section"
    );
    assert!(
        manager.models_config().providers.is_empty(),
        "ParseFailed must yield the empty default"
    );
}

/// Hot-update path (`update_section_cache`, used by hot reload and
/// pending-restart apply): after `load`, the cache follows the freshly
/// written section value — `models_config()` returns the new value, not
/// the load-time one (Step 1.23 — reload wiring).
#[test]
fn test_models_config_update_section_cache_returns_new_value() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_without_models(tmp.path()).unwrap();
    fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai":{"models":[{"id":"m1"}]}}}"#,
    )
    .unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().expect("initial load");
    assert!(manager.models_config().providers.contains_key("openai"));

    let new_value = serde_json::json!({
        "providers": {"glm": {"models": [{"id": "glm-4-plus"}]}}
    });
    manager.update_section_cache(
        ConfigSection::Models,
        tmp.path().join("models.json"),
        new_value.clone(),
    );

    assert_eq!(manager.section(ConfigSection::Models), Some(new_value));
    let models = manager.models_config();
    assert!(
        !models.providers.contains_key("openai"),
        "stale load-time value must be replaced"
    );
    assert!(
        models.providers.contains_key("glm"),
        "hot-updated value must be cached"
    );
}
