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
}

/// Test: reload_section parse failure keeps old value.
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

/// Test: no event is emitted if the file does not exist (early IoError return).
#[test]
fn test_reload_section_file_not_found_no_event() {
    let tmp = tempfile::tempdir().unwrap();
    // Don't create any config files
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    let mut rx = manager.subscribe_config_changes();

    let result = manager.reload_section(ConfigSection::System, None);
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
