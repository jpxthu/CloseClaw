//! Tests for ConfigManager hot-reload methods (reload_section / reload_agents).

use crate::common::test_helpers::write_mandatory_configs;
use crate::config::events::ConfigChangeEvent;
use crate::config::manager::*;
use crate::config::manager_reload::SectionValidator;
use std::fs;

/// Helper: set up a valid config directory with all mandatory JSON files.
fn setup_config_dir_at(dir: &std::path::Path) {
    write_mandatory_configs(dir).unwrap();
}

// ---------------------------------------------------------------------------
// reload_section — basic (no validator)
// ---------------------------------------------------------------------------

/// Test: reload_section with valid JSON updates the in-memory cache.
#[test]
fn test_reload_section_success() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::System).unwrap();
    assert_eq!(before["version"], "1.0");

    // Overwrite the canonical file with new content
    fs::write(
        tmp.path().join("system.json"),
        r#"{"version": "9.9", "updated": true}"#,
    )
    .unwrap();
    manager.reload_section(ConfigSection::System, None).unwrap();

    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "9.9");
    assert_eq!(after["updated"], true);
}

/// Test: reload_section with invalid JSON returns an error and leaves the
/// cache unchanged.
#[test]
fn test_reload_section_invalid_json() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::System).unwrap();
    assert_eq!(before["version"], "1.0");

    // Corrupt the canonical file
    fs::write(tmp.path().join("system.json"), "not json").unwrap();
    let result = manager.reload_section(ConfigSection::System, None);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ParseError { .. }
    ));

    // In-memory cache must still be the old value
    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "1.0");

    // File is rolled back to the in-memory old value (last known good state)
    let file_content = fs::read_to_string(tmp.path().join("system.json")).unwrap();
    let restored: serde_json::Value = serde_json::from_str(&file_content).unwrap();
    assert_eq!(
        restored["version"], "1.0",
        "file should be rolled back to the in-memory old value"
    );
}

// ---------------------------------------------------------------------------
// reload_section — validator callback
// ---------------------------------------------------------------------------

/// Test: reload_section with passing validator updates in-memory cache.
#[test]
fn test_reload_section_validator_success() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    fs::write(tmp.path().join("system.json"), r#"{"version": "2.0"}"#).unwrap();

    let validator: &SectionValidator = &|v: &serde_json::Value| {
        if v.get("version").is_some() {
            Ok(())
        } else {
            Err("version required".into())
        }
    };

    manager
        .reload_section(ConfigSection::System, Some(validator))
        .unwrap();

    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "2.0");
}

/// Test: reload_section with failing validator rejects the reload, keeps old
/// value in memory.
#[test]
fn test_reload_section_validator_failure_keeps_old_value() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::System).unwrap();
    assert_eq!(before["version"], "1.0");

    // Write content that will fail validation
    fs::write(
        tmp.path().join("system.json"),
        r#"{"version": "2.0", "bad_field": true}"#,
    )
    .unwrap();

    let validator: &SectionValidator = &|v: &serde_json::Value| {
        if v.get("bad_field").is_some() {
            Err("bad_field is not allowed".into())
        } else {
            Ok(())
        }
    };

    let result = manager.reload_section(ConfigSection::System, Some(validator));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ValidationError { .. }
    ));

    // In-memory cache must still be the old value
    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "1.0");
    assert!(after.get("bad_field").is_none());

    // File is rolled back to the in-memory old value (last known good state)
    let file_content = fs::read_to_string(tmp.path().join("system.json")).unwrap();
    let restored: serde_json::Value = serde_json::from_str(&file_content).unwrap();
    assert_eq!(restored["version"], "1.0");
    assert!(
        restored.get("bad_field").is_none(),
        "file should be rolled back to the in-memory old value"
    );
}

/// Test: reload_section parse failure keeps old value and restores file.
#[test]
fn test_reload_section_parse_failure_keeps_old_value() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Corrupt the canonical file
    fs::write(tmp.path().join("system.json"), "not json").unwrap();

    let result = manager.reload_section(ConfigSection::System, None);
    assert!(result.is_err());

    // In-memory unchanged
    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "1.0");

    // File is rolled back to the in-memory old value (last known good state)
    let file_content = fs::read_to_string(tmp.path().join("system.json")).unwrap();
    let restored: serde_json::Value = serde_json::from_str(&file_content).unwrap();
    assert_eq!(
        restored["version"], "1.0",
        "file should be rolled back to the in-memory old value"
    );
}

// ---------------------------------------------------------------------------
// reload_section — event emission
// ---------------------------------------------------------------------------

/// Test: successful reload emits a Reloaded event via the broadcast channel.
#[test]
fn test_reload_section_success_emits_reloaded_event() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let mut rx = manager.subscribe_config_changes();

    fs::write(tmp.path().join("models.json"), r#"{"version": "3.0"}"#).unwrap();
    manager.reload_section(ConfigSection::Models, None).unwrap();

    let event = rx.try_recv().expect("should receive a Reloaded event");
    match event {
        ConfigChangeEvent::Reloaded { section } => {
            assert_eq!(section, ConfigSection::Models);
        }
        other => panic!("expected Reloaded, got {:?}", other),
    }
}

/// Test: validation failure emits a Failed event with the error message.
#[test]
fn test_reload_section_validation_failure_emits_failed_event() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let mut rx = manager.subscribe_config_changes();

    fs::write(tmp.path().join("models.json"), r#"{"version": "3.0"}"#).unwrap();

    let validator: &SectionValidator = &|_: &serde_json::Value| Err("reject all".into());
    let _ = manager.reload_section(ConfigSection::Models, Some(validator));

    let event = rx.try_recv().expect("should receive a Failed event");
    match event {
        ConfigChangeEvent::Failed { section, error } => {
            assert_eq!(section, ConfigSection::Models);
            assert_eq!(error, "reject all");
        }
        other => panic!("expected Failed, got {:?}", other),
    }
}

/// Test: parse failure emits a Failed event.
#[test]
fn test_reload_section_parse_failure_emits_failed_event() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let mut rx = manager.subscribe_config_changes();

    fs::write(tmp.path().join("gateway.json"), "not json").unwrap();
    let _ = manager.reload_section(ConfigSection::Gateway, None);

    let event = rx.try_recv().expect("should receive a Failed event");
    match event {
        ConfigChangeEvent::Failed { section, error } => {
            assert_eq!(section, ConfigSection::Gateway);
            assert!(error.contains("expected"), "error: {}", error);
        }
        other => panic!("expected Failed, got {:?}", other),
    }
}

/// Test: file not found emits a Failed event (IoError path).
#[test]
fn test_reload_section_file_not_found_emits_failed_event() {
    let tmp = tempfile::tempdir().unwrap();
    // Don't create any config files
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    let mut rx = manager.subscribe_config_changes();

    let result = manager.reload_section(ConfigSection::System, None);
    assert!(result.is_err());

    // IoError path should emit a Failed event
    let event = rx.try_recv().expect("should receive a Failed event");
    match event {
        ConfigChangeEvent::Failed { section, error } => {
            assert_eq!(section, ConfigSection::System);
            assert!(error.contains("No such file") || error.contains("os error"));
        }
        other => panic!("expected Failed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// reload_agents (existing)
// ---------------------------------------------------------------------------

/// Test: reload_agents succeeds when agents/ directory has valid agent configs.
#[test]
fn test_reload_agents_success() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    setup_config_dir_at(&config_dir);

    let agents_json = r#"{ "version": "1.0", "agents": ["alpha"] }"#;
    let agents_dir = config_dir.join("config");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(agents_dir.join("agents.json"), agents_json).unwrap();

    let agent_dir = config_dir.join("agents").join("alpha");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("config.json"),
        r#"{ "id": "alpha", "name": "Alpha Agent" }"#,
    )
    .unwrap();

    let manager = ConfigManager::new(config_dir).unwrap();
    manager.reload_agents().unwrap();

    let agents = manager.agents();
    assert!(agents.contains_key("alpha"));
    assert_eq!(agents["alpha"].id, "alpha");
}

/// Test: when reload_agents() fails, snapshot_agents + restore_agents
/// preserves the original in-memory state.
#[test]
fn test_reload_agents_failure_rollback_via_snapshot_restore() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    setup_config_dir_at(&config_dir);

    // Set up a valid agent: alpha
    let agents_json = r#"{ "agents": ["alpha"] }"#;
    let config_agents_dir = config_dir.join("config");
    fs::create_dir_all(&config_agents_dir).unwrap();
    fs::write(config_agents_dir.join("agents.json"), agents_json).unwrap();

    let agent_dir = config_dir.join("agents").join("alpha");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("config.json"),
        r#"{ "id": "alpha", "name": "Alpha" }"#,
    )
    .unwrap();

    let manager = ConfigManager::new(config_dir.clone()).unwrap();
    manager.load_agents(None).unwrap();

    // Verify alpha is loaded
    assert!(manager.agents().contains_key("alpha"));

    // Take a snapshot
    let (old_agents, old_permissions) = manager.snapshot_agents();
    assert_eq!(old_agents.len(), 1);
    assert!(old_agents.contains_key("alpha"));

    // Corrupt agents.json so reload_agents() fails
    fs::write(config_agents_dir.join("agents.json"), "not json").unwrap();
    let result = manager.reload_agents();
    assert!(result.is_err());

    // Restore from snapshot
    manager.restore_agents(old_agents, old_permissions);

    // Verify alpha is still present (rollback succeeded)
    let agents = manager.agents();
    assert!(agents.contains_key("alpha"));
    assert_eq!(agents["alpha"].id, "alpha");
}

// =========================================================================
// Step 1.2 — reload_section backup/restore edge cases
// =========================================================================

// ---------------------------------------------------------------------------
// reload_section — IO failure edge cases
// ---------------------------------------------------------------------------

/// Test: reload_section when file doesn't exist and no previous load was done.
/// In-memory cache should remain None and error should be returned.
#[test]
fn test_reload_section_io_failure_no_prior_load() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    // Don't call load() — sections map is empty

    let result = manager.reload_section(ConfigSection::System, None);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::IoError { .. }
    ));

    // In-memory should still be None (no prior load)
    assert!(manager.section(ConfigSection::System).is_none());
}

/// Test: reload_section after successful load, then file is removed.
/// In-memory cache should be unchanged.
#[test]
fn test_reload_section_io_failure_after_load() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::System).unwrap();
    assert_eq!(before["version"], "1.0");

    // Remove the file
    fs::remove_file(tmp.path().join("system.json")).unwrap();

    let result = manager.reload_section(ConfigSection::System, None);
    assert!(result.is_err());

    // In-memory unchanged
    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "1.0");
}

// ---------------------------------------------------------------------------
// reload_section — backup directory edge cases
// ---------------------------------------------------------------------------

/// Test: ConfigManager::new when backup dir path is a file returns error.
#[test]
fn test_config_manager_new_backup_path_is_file() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("cfg");
    fs::create_dir_all(&config_dir).unwrap();
    // Make .backups a file so create_dir_all fails
    fs::write(config_dir.join(".backups"), "not a dir").unwrap();

    let result = ConfigManager::new(config_dir);
    assert!(result.is_err(), "should fail when .backups is a file");
}

/// Test: ConfigManager::new when config dir is a file (parent not a dir).
#[test]
fn test_config_manager_new_config_dir_is_file() {
    let tmp = tempfile::tempdir().unwrap();
    let config_file = tmp.path().join("config_as_file");
    fs::write(&config_file, "not a dir").unwrap();

    // Should return error, not panic
    let result = ConfigManager::new(config_file);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// reload_section — multiple sequential reloads
// ---------------------------------------------------------------------------

/// Test: multiple sequential reloads on the same section work correctly.
#[test]
fn test_reload_section_multiple_sequential_reloads() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // First reload
    fs::write(tmp.path().join("system.json"), r#"{"version": "2.0"}"#).unwrap();
    manager.reload_section(ConfigSection::System, None).unwrap();
    assert_eq!(
        manager.section(ConfigSection::System).unwrap()["version"],
        "2.0"
    );

    // Second reload
    fs::write(tmp.path().join("system.json"), r#"{"version": "3.0"}"#).unwrap();
    manager.reload_section(ConfigSection::System, None).unwrap();
    assert_eq!(
        manager.section(ConfigSection::System).unwrap()["version"],
        "3.0"
    );

    // Third reload with failure — should revert to 3.0
    fs::write(tmp.path().join("system.json"), "bad json").unwrap();
    let result = manager.reload_section(ConfigSection::System, None);
    assert!(result.is_err());
    assert_eq!(
        manager.section(ConfigSection::System).unwrap()["version"],
        "3.0"
    );
}

/// Test: reloads on different sections are independent.
#[test]
fn test_reload_section_independent_sections() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Update System successfully
    fs::write(tmp.path().join("system.json"), r#"{"version": "2.0"}"#).unwrap();
    manager.reload_section(ConfigSection::System, None).unwrap();

    // Corrupt Models
    fs::write(tmp.path().join("models.json"), "bad json").unwrap();
    let result = manager.reload_section(ConfigSection::Models, None);
    assert!(result.is_err());

    // System should still be updated
    assert_eq!(
        manager.section(ConfigSection::System).unwrap()["version"],
        "2.0"
    );
    // Models should be unchanged
    assert_eq!(
        manager.section(ConfigSection::Models).unwrap()["version"],
        "1.0"
    );
}

// ---------------------------------------------------------------------------
// reload_section — file restore content correctness
// ---------------------------------------------------------------------------

/// Test: after parse failure, the restored file content is valid JSON
/// matching the old in-memory value.
#[test]
fn test_reload_section_restored_file_is_valid_json() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Corrupt the file
    fs::write(tmp.path().join("gateway.json"), "not json!!!").unwrap();
    let result = manager.reload_section(ConfigSection::Gateway, None);
    assert!(result.is_err());

    // File is rolled back to the in-memory old value (last known good state)
    let file_content = fs::read_to_string(tmp.path().join("gateway.json")).unwrap();
    let restored: serde_json::Value = serde_json::from_str(&file_content).unwrap();
    assert_eq!(
        restored["version"], "1.0",
        "file should be rolled back to the in-memory old value"
    );
}

/// Test: after validation failure, the restored file content is valid JSON
/// matching the old in-memory value.
#[test]
fn test_reload_section_validation_failure_restored_file_is_valid_json() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Write valid JSON that fails validation
    fs::write(tmp.path().join("gateway.json"), r#"{"version": "bad"}"#).unwrap();

    let validator: &SectionValidator = &|v: &serde_json::Value| {
        if v.get("version").and_then(|v| v.as_str()) == Some("1.0") {
            Ok(())
        } else {
            Err("version must be 1.0".into())
        }
    };

    let result = manager.reload_section(ConfigSection::Gateway, Some(validator));
    assert!(result.is_err());

    // File is rolled back to the in-memory old value (last known good state)
    let file_content = fs::read_to_string(tmp.path().join("gateway.json")).unwrap();
    let restored: serde_json::Value = serde_json::from_str(&file_content).unwrap();
    assert_eq!(restored["version"], "1.0");
}

/// Test: when in-memory section was never loaded (None),
/// rollback does not change the file when no backup is available.
#[test]
fn test_reload_section_no_prior_value_no_restore_write() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    // Don't load — System section is not in memory

    // Write invalid JSON
    fs::write(tmp.path().join("system.json"), "bad json").unwrap();
    let result = manager.reload_section(ConfigSection::System, None);
    assert!(result.is_err());

    // File should still contain the bad JSON (no restore possible)
    let file_content = fs::read_to_string(tmp.path().join("system.json")).unwrap();
    assert_eq!(file_content, "bad json");
}

// ---------------------------------------------------------------------------
// reload_section — combined validator + file restore
// ---------------------------------------------------------------------------

/// Test: validator failure after valid parse restores file to old content.
#[test]
fn test_reload_section_validator_failure_restores_file_after_valid_parse() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::Plugins).unwrap();
    assert_eq!(before["version"], "1.0");

    // Write valid JSON that will fail validation
    fs::write(
        tmp.path().join("plugins.json"),
        r#"{"version": "9.0", "banned": true}"#,
    )
    .unwrap();

    let validator: &SectionValidator = &|v: &serde_json::Value| {
        if v.get("banned").is_some() {
            Err("banned field not allowed".into())
        } else {
            Ok(())
        }
    };

    let result = manager.reload_section(ConfigSection::Plugins, Some(validator));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ValidationError { .. }
    ));

    // In-memory unchanged
    let after = manager.section(ConfigSection::Plugins).unwrap();
    assert_eq!(after["version"], "1.0");

    // File is rolled back to the in-memory old value (last known good state)
    let file_content = fs::read_to_string(tmp.path().join("plugins.json")).unwrap();
    let restored: serde_json::Value = serde_json::from_str(&file_content).unwrap();
    assert_eq!(restored["version"], "1.0");
    assert!(
        restored.get("banned").is_none(),
        "file should be rolled back to the in-memory old value"
    );
}

/// Test: reload_section success does not create backup files.
/// On success the file content stays as-is (new content).
#[test]
fn test_reload_section_success_file_contains_new_content() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let new_content = r#"{"version": "5.0", "extra": [1, 2, 3]}"#;
    fs::write(tmp.path().join("system.json"), new_content).unwrap();
    manager.reload_section(ConfigSection::System, None).unwrap();

    // File on disk should still contain the new content (not reverted)
    let file_content = fs::read_to_string(tmp.path().join("system.json")).unwrap();
    assert!(file_content.contains("5.0"));
    assert!(file_content.contains("extra"));
}

// =========================================================================
// reload_agents (existing)
// =========================================================================

/// Test: restore_agents correctly replaces current state with snapshot.
#[test]
fn test_restore_agents_replaces_current_state() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    setup_config_dir_at(&config_dir);

    let config_agents_dir = config_dir.join("config");
    fs::create_dir_all(&config_agents_dir).unwrap();

    // Set up agent alpha
    fs::write(
        config_agents_dir.join("agents.json"),
        r#"{ "agents": ["alpha"] }"#,
    )
    .unwrap();
    let alpha_dir = config_dir.join("agents").join("alpha");
    fs::create_dir_all(&alpha_dir).unwrap();
    fs::write(
        alpha_dir.join("config.json"),
        r#"{ "id": "alpha", "name": "Alpha" }"#,
    )
    .unwrap();

    let manager = ConfigManager::new(config_dir.clone()).unwrap();
    manager.load_agents(None).unwrap();

    // Snapshot the initial state (alpha only)
    let snapshot = manager.snapshot_agents();

    // Now add agent beta manually by writing files and reloading
    fs::write(
        config_agents_dir.join("agents.json"),
        r#"{ "agents": ["alpha", "beta"] }"#,
    )
    .unwrap();
    let beta_dir = config_dir.join("agents").join("beta");
    fs::create_dir_all(&beta_dir).unwrap();
    fs::write(
        beta_dir.join("config.json"),
        r#"{ "id": "beta", "name": "Beta" }"#,
    )
    .unwrap();
    manager.reload_agents().unwrap();
    assert!(manager.agents().contains_key("beta"));

    // Restore the snapshot (should remove beta)
    manager.restore_agents(snapshot.0, snapshot.1);

    let agents = manager.agents();
    assert!(agents.contains_key("alpha"));
    assert!(
        !agents.contains_key("beta"),
        "beta should not exist after restore"
    );
}

// =========================================================================
// Step 1.5 — validator integration tests
// =========================================================================

// ---------------------------------------------------------------------------
// reload_section with ConfigSection default_validator
// ---------------------------------------------------------------------------

/// Test: reload_section with the default Models validator accepts valid JSON
/// and rejects non-array `models` field.
#[test]
fn test_reload_section_default_validator_models() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Valid: models is an array
    fs::write(
        tmp.path().join("models.json"),
        r#"{"models":[{"id":"m1"}]}"#,
    )
    .unwrap();
    let v = ConfigSection::Models.default_validator();
    manager
        .reload_section(ConfigSection::Models, Some(&*v))
        .unwrap();
    assert!(
        manager.section(ConfigSection::Models).unwrap()["models"].is_array(),
        "models field should be an array after successful reload"
    );
}

/// Test: reload_section with default Models validator rejects non-array.
#[test]
fn test_reload_section_default_validator_models_rejects_non_array() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::Models).unwrap();

    // Invalid: models is a string, not an array
    fs::write(tmp.path().join("models.json"), r#"{"models":"not array"}"#).unwrap();
    let v = ConfigSection::Models.default_validator();
    let result = manager.reload_section(ConfigSection::Models, Some(&*v));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ValidationError { .. }
    ));

    // In-memory unchanged
    let after = manager.section(ConfigSection::Models).unwrap();
    assert_eq!(
        before, after,
        "models should be unchanged after validation failure"
    );
}

/// Test: reload_section with default Gateway validator accepts valid JSON.
#[test]
fn test_reload_section_default_validator_gateway() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    fs::write(tmp.path().join("gateway.json"), r#"{"port":3000}"#).unwrap();
    let v = ConfigSection::Gateway.default_validator();
    manager
        .reload_section(ConfigSection::Gateway, Some(&*v))
        .unwrap();
    assert_eq!(
        manager.section(ConfigSection::Gateway).unwrap()["port"],
        3000
    );
}

/// Test: reload_section with default Gateway validator rejects non-object.
#[test]
fn test_reload_section_default_validator_gateway_rejects_non_object() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    fs::write(tmp.path().join("gateway.json"), r#"[1,2,3]"#).unwrap();
    let v = ConfigSection::Gateway.default_validator();
    let result = manager.reload_section(ConfigSection::Gateway, Some(&*v));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ValidationError { .. }
    ));
}

/// Test: reload_section with default System validator accepts and rejects.
#[test]
fn test_reload_section_default_validator_system() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Valid
    fs::write(tmp.path().join("system.json"), r#"{"version":"2.0"}"#).unwrap();
    let v = ConfigSection::System.default_validator();
    manager
        .reload_section(ConfigSection::System, Some(&*v))
        .unwrap();
    assert_eq!(
        manager.section(ConfigSection::System).unwrap()["version"],
        "2.0"
    );

    // Invalid: array at top level
    fs::write(tmp.path().join("system.json"), r#"[1]"#).unwrap();
    let v = ConfigSection::System.default_validator();
    let result = manager.reload_section(ConfigSection::System, Some(&*v));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ValidationError { .. }
    ));
}
