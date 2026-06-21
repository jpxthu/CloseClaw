//! Tests for ConfigManager hot-reload methods (reload_section / reload_agents).

use crate::config::manager::*;
use std::fs;

/// Helper: set up a valid config directory with all mandatory JSON files.
fn setup_config_dir_at(dir: &std::path::Path) {
    fs::write(dir.join("models.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("channels.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("gateway.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("plugins.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("system.json"), r#"{"version": "1.0"}"#).unwrap();
}

/// Test: reload_section with valid JSON updates the in-memory cache.
#[test]
fn test_reload_section_success() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::System).unwrap();
    assert_eq!(before["version"], "1.0");

    let new_json = r#"{"version": "9.9", "updated": true}"#;
    manager
        .reload_section(ConfigSection::System, new_json, None)
        .unwrap();

    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "9.9");
    assert_eq!(after["updated"], true);
}

/// Test: reload_section with invalid JSON returns an error and leaves the cache unchanged.
#[test]
fn test_reload_section_invalid_json() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let before = manager.section(ConfigSection::System).unwrap();
    assert_eq!(before["version"], "1.0");

    let result = manager.reload_section(ConfigSection::System, "not json", None);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ParseError { .. }
    ));

    let after = manager.section(ConfigSection::System).unwrap();
    assert_eq!(after["version"], "1.0");
}

/// Test: reload_agents succeeds when agents/ directory has valid agent configs.
///
/// Note: `load_agents` resolves the user agents directory as
/// `config_dir.join("agents")`, so the agent directory must
/// be inside the config directory.
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
