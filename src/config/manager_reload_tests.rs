//! Tests for ConfigManager hot-reload methods (reload_section / reload_agents).

use crate::config::events::ConfigChangeEvent;
use crate::config::manager::*;
use crate::config::manager_reload::SectionValidator;
use std::fs;

/// Helper: set up a valid config directory with all mandatory JSON files.
fn setup_config_dir_at(dir: &std::path::Path) {
    fs::write(dir.join("models.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("channels.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("gateway.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("plugins.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("system.json"), r#"{"version": "1.0"}"#).unwrap();
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

    let new_content = r#"{"version": "9.9", "updated": true}"#;
    let new_file = tmp.path().join("new_system.json");
    fs::write(&new_file, new_content).unwrap();
    manager
        .reload_section(ConfigSection::System, &new_file, None)
        .unwrap();

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

    let bad_file = tmp.path().join("bad.json");
    fs::write(&bad_file, "not json").unwrap();
    let result = manager.reload_section(ConfigSection::System, &bad_file, None);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ParseError { .. }
    ));

    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "1.0");
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

    let new_file = tmp.path().join("new_system.json");
    fs::write(&new_file, r#"{"version": "2.0"}"#).unwrap();

    let validator: &SectionValidator = &|v: &serde_json::Value| {
        if v.get("version").is_some() {
            Ok(())
        } else {
            Err("version required".into())
        }
    };

    manager
        .reload_section(ConfigSection::System, &new_file, Some(validator))
        .unwrap();

    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "2.0");
}

/// Test: reload_section with failing validator rejects the reload, keeps old
/// value in memory, and rolls back the file.
#[test]
fn test_reload_section_validator_failure_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::System).unwrap();
    assert_eq!(before["version"], "1.0");

    let system_path = tmp.path().join("system.json");
    let original_content = fs::read_to_string(&system_path).unwrap();

    let new_file = tmp.path().join("new_system.json");
    fs::write(&new_file, r#"{"version": "2.0", "bad_field": true}"#).unwrap();

    let validator: &SectionValidator = &|v: &serde_json::Value| {
        if v.get("bad_field").is_some() {
            Err("bad_field is not allowed".into())
        } else {
            Ok(())
        }
    };

    let result = manager.reload_section(ConfigSection::System, &new_file, Some(validator));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ValidationError { .. }
    ));

    // In-memory cache must still be the old value
    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "1.0");
    assert!(after.get("bad_field").is_none());

    // File must be rolled back to original content
    let file_after = fs::read_to_string(&system_path).unwrap();
    assert_eq!(file_after, original_content);
}

/// Test: reload_section parse failure rolls back the file and keeps old value.
#[test]
fn test_reload_section_parse_failure_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let system_path = tmp.path().join("system.json");
    let original_content = fs::read_to_string(&system_path).unwrap();

    let bad_file = tmp.path().join("bad.json");
    fs::write(&bad_file, "not json").unwrap();

    let result = manager.reload_section(ConfigSection::System, &bad_file, None);
    assert!(result.is_err());

    // In-memory unchanged
    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "1.0");

    // File rolled back to original
    let file_after = fs::read_to_string(&system_path).unwrap();
    assert_eq!(file_after, original_content);
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

    let new_file = tmp.path().join("new_models.json");
    fs::write(&new_file, r#"{"version": "3.0"}"#).unwrap();
    manager
        .reload_section(ConfigSection::Models, &new_file, None)
        .unwrap();

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

    let new_file = tmp.path().join("new_models.json");
    fs::write(&new_file, r#"{"version": "3.0"}"#).unwrap();

    let validator: &SectionValidator = &|_: &serde_json::Value| Err("reject all".into());
    let _ = manager.reload_section(ConfigSection::Models, &new_file, Some(validator));

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

    let bad_file = tmp.path().join("bad.json");
    fs::write(&bad_file, "not json").unwrap();
    let _ = manager.reload_section(ConfigSection::Gateway, &bad_file, None);

    let event = rx.try_recv().expect("should receive a Failed event");
    match event {
        ConfigChangeEvent::Failed { section, error } => {
            assert_eq!(section, ConfigSection::Gateway);
            assert!(error.contains("expected"), "error: {}", error);
        }
        other => panic!("expected Failed, got {:?}", other),
    }
}

/// Test: no event is emitted if the file does not exist (early IoError return).
#[test]
fn test_reload_section_file_not_found_no_event() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let mut rx = manager.subscribe_config_changes();

    let missing = tmp.path().join("does_not_exist.json");
    let result = manager.reload_section(ConfigSection::System, &missing, None);
    assert!(result.is_err());

    // IoError path returns before any event is emitted
    assert!(rx.try_recv().is_err());
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
