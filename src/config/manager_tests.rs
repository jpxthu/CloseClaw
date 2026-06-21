//! Tests for ConfigManager

use super::*;
use std::fs;

// ---------------------------------------------------------------------------
// write_atomically
// ---------------------------------------------------------------------------

#[test]
fn test_write_atomically_normal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.json");
    write_atomically(&path, b"{\"test\": true}").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"{\"test\": true}");
}

#[test]
fn test_write_atomically_nested_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a/b/c/test.json");
    write_atomically(&path, b"{\"nested\": true}").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"{\"nested\": true}");
}

#[test]
fn test_write_atomically_cleanup_on_failure() {
    let tmp = tempfile::tempdir().unwrap();
    // Target path is a directory — rename to it will fail.
    // The temp file must be cleaned up.
    let dir = tmp.path().join("subdir");
    fs::create_dir_all(&dir).unwrap();
    let result = write_atomically(&dir, b"test");
    assert!(result.is_err()); // rename must fail
                              // No .tmp.* files should remain in tmp
    let entries: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| n.to_str().unwrap_or("").starts_with(".tmp."))
        .collect();
    assert!(
        entries.is_empty(),
        "temp files not cleaned up: {:?}",
        entries
    );
}

// ---------------------------------------------------------------------------
// ConfigSection Display
// ---------------------------------------------------------------------------

#[test]
fn test_config_section_display() {
    assert_eq!(ConfigSection::Models.to_string(), "models.json");
    assert_eq!(ConfigSection::Channels.to_string(), "channels.json");
    assert_eq!(ConfigSection::Gateway.to_string(), "gateway.json");
    assert_eq!(ConfigSection::Plugins.to_string(), "plugins.json");
    assert_eq!(ConfigSection::System.to_string(), "system.json");
    assert_eq!(ConfigSection::Credentials.to_string(), "credentials.json");
}

// ---------------------------------------------------------------------------
// ConfigLoadError / ConfigWriteError Display and From
// ---------------------------------------------------------------------------

#[test]
fn test_config_load_error_display() {
    let err = ConfigLoadError::ConfigFileNotFound(PathBuf::from("/missing.json"));
    assert!(err.to_string().contains("missing.json"));
    let err = ConfigLoadError::ParseError {
        path: PathBuf::from("/bad.json"),
        error: "unexpected token".to_string(),
    };
    assert!(err.to_string().contains("unexpected token"));
}

#[test]
fn test_config_write_error_display() {
    let err =
        ConfigWriteError::ValidationFailed("models.json".to_string(), "bad value".to_string());
    assert!(err.to_string().contains("bad value"));
    let err = ConfigWriteError::WriteFailed {
        path: PathBuf::from("/path.json"),
        error: "disk full".to_string(),
    };
    assert!(err.to_string().contains("disk full"));
}

#[test]
fn test_config_validation_error_display() {
    let err = ConfigValidationError {
        path: PathBuf::from("/test.json"),
        message: "version required".to_string(),
    };
    assert!(err.to_string().contains("version required"));
    assert!(err.to_string().contains("/test.json"));
}

// ---------------------------------------------------------------------------
// ConfigManager — corrupted file + rollback recovery
// ---------------------------------------------------------------------------

/// Test: corrupted mandatory file + valid backup → load succeeds via rollback.
#[test]
fn test_config_manager_load_corrupted_with_backup_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Update creates a backup of the current models.json
    manager
        .update(
            ConfigSection::Models,
            serde_json::json!({"version": "2.0"}),
            |_| Ok(()),
        )
        .unwrap();

    // Re-load so the in-memory cache matches what the backup will restore
    manager.load().unwrap();

    // Verify in-memory cache before corrupting
    let section_before = manager.section(ConfigSection::Models).unwrap();
    assert_eq!(
        section_before["version"], "2.0",
        "cache should be 2.0 before corruption"
    );

    // Corrupt models.json — JSON parse will fail
    let models_path = tmp.path().join("models.json");
    fs::write(&models_path, "not valid json {{").unwrap();

    // Load should succeed because rollback recovers from the backup made above
    manager.load().unwrap();

    let section = manager.section(ConfigSection::Models).unwrap();
    // The backup was created BEFORE the update to version 2.0, so it contains version 1.0
    // Rollback should restore the backup, which is version 1.0
    assert_eq!(section["version"], "1.0");
}

/// Test: corrupted mandatory file + backup that is also corrupted → load fails.
#[test]
fn test_config_manager_load_corrupted_backup_also_corrupted() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Update creates a backup of the current models.json
    manager
        .update(
            ConfigSection::Models,
            serde_json::json!({"version": "2.0"}),
            |_| Ok(()),
        )
        .unwrap();

    // Corrupt the backup itself: find backup path and overwrite with bad JSON
    let models_path = tmp.path().join("models.json");
    let backup_dir = tmp.path().join(".backups");
    let backup_path = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with("models."))
                .unwrap_or(false)
        })
        .expect("should find a models backup");
    fs::write(&backup_path, "also not valid json {{").unwrap();

    // Corrupt models.json
    fs::write(&models_path, "not valid json {{").unwrap();

    // Load must fail because even the recovered backup is unparseable
    let result = manager.load();
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ParseError { .. }
    ));
}

/// Test: corrupted mandatory file + no backup available → load fails.
#[test]
fn test_config_manager_load_corrupted_no_backup() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Ensure .backups directory does not exist
    let backup_dir = tmp.path().join(".backups");
    fs::remove_dir_all(&backup_dir).ok();

    // Corrupt models.json
    let models_path = tmp.path().join("models.json");
    fs::write(&models_path, "not valid json {{").unwrap();

    // Load must fail because there is no backup to recover from
    let result = manager.load();
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ParseError { .. }
    ));
}

/// Test: normal load flow is unaffected when files are valid.
#[test]
fn test_config_manager_load_normal_unaffected() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // All mandatory sections should be loaded successfully
    for section in &[
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ] {
        assert!(
            manager.section(*section).is_some(),
            "section {:?} missing",
            section
        );
    }
}

/// Test: credentials directory does not exist → load still succeeds.
#[test]
fn test_config_manager_load_credentials_missing_dir() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // credentials/ directory does not exist — load should still succeed
    assert!(manager.section(ConfigSection::Models).is_some());
}

/// Test: credentials directory contains a corrupted JSON file → load still succeeds.
#[test]
fn test_config_manager_load_credentials_corrupted_file() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);

    // Create credentials/ directory with a bad JSON file
    let creds_dir = tmp.path().join("credentials");
    fs::create_dir_all(&creds_dir).unwrap();
    fs::write(creds_dir.join("openai.json"), "not valid json {{").unwrap();

    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Mandatory sections should load successfully despite corrupted credentials file
    assert!(manager.section(ConfigSection::Models).is_some());
}

// ---------------------------------------------------------------------------
// ConfigManager — happy path
// ---------------------------------------------------------------------------

fn setup_config_dir(tmp: &tempfile::TempDir) {
    setup_config_dir_at(tmp.path());
}

fn setup_config_dir_at(dir: &std::path::Path) {
    fs::write(dir.join("models.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("channels.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("gateway.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("plugins.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(dir.join("system.json"), r#"{"version": "1.0"}"#).unwrap();
}

#[test]
fn test_config_manager_new() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    assert!(manager.section(ConfigSection::Models).is_none());
}

#[test]
fn test_config_manager_load() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();
    for section in &[
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ] {
        assert!(
            manager.section(*section).is_some(),
            "section {:?} missing",
            section
        );
    }
}

#[test]
fn test_config_manager_load_missing_file() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("models.json"), r#"{"version": "1.0"}"#).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    let result = manager.load();
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ConfigFileNotFound(_)
    ));
}

#[test]
fn test_config_manager_load_missing_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let empty_dir = tmp.path().join("empty_config");
    fs::create_dir_all(&empty_dir).unwrap();
    let manager = ConfigManager::new(empty_dir).unwrap();
    let result = manager.load();
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigLoadError::ConfigFileNotFound(_)
    ));
}

#[test]
fn test_config_manager_update() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let new_value = serde_json::json!({"version": "2.0", "models": ["gpt-4"]});
    let validator = |v: &serde_json::Value| {
        if v.get("version").is_some() {
            Ok(())
        } else {
            Err(ConfigValidationError {
                path: PathBuf::from("models.json"),
                message: "version required".to_string(),
            })
        }
    };
    manager
        .update(ConfigSection::Models, new_value, validator)
        .unwrap();

    let section = manager.section(ConfigSection::Models).unwrap();
    assert_eq!(section["version"], "2.0");
    assert_eq!(section["models"][0], "gpt-4");
}

#[test]
fn test_config_manager_update_validation_failure() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let validator = |_: &serde_json::Value| {
        Err(ConfigValidationError {
            path: PathBuf::from("models.json"),
            message: "always fail".to_string(),
        })
    };
    let result = manager.update(
        ConfigSection::Models,
        serde_json::json!({"bad": "value"}),
        validator,
    );
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigWriteError::ValidationFailed(_, _)
    ));
}

#[test]
fn test_config_manager_list_configs() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let infos = manager.list_configs();
    assert!(!infos.is_empty());
    let models_info = infos
        .iter()
        .find(|i| i.path.contains("models.json"))
        .unwrap();
    assert_eq!(models_info.version, "1.0");
    assert!(models_info.last_modified.is_some());
}

#[test]
fn test_config_manager_update_backup_failure() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Replace the backup directory with a file so backup creation fails.
    let backup_dir = tmp.path().join(".backups");
    fs::remove_dir_all(&backup_dir).ok();
    fs::write(&backup_dir, b"not a directory").unwrap();

    let new_value = serde_json::json!({"version": "3.0"});
    let validator = |_: &serde_json::Value| Ok(());
    let result = manager.update(ConfigSection::Models, new_value, validator);

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ConfigWriteError::BackupFailed { .. }
    ));
}

// =====================================================================
// Step 1.2 — ConfigManager.load() section-population tests
// =====================================================================

/// Test: load() populates memory cache with exact JSON values for all
/// 5 mandatory sections.
#[test]
fn test_load_populates_all_five_sections_with_values() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir_at(tmp.path());
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();

    let mandatory_sections = [
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ];

    // Before load: all sections should be None
    for section in &mandatory_sections {
        assert!(
            manager.section(*section).is_none(),
            "section {:?} should be None before load",
            section
        );
    }

    manager.load().unwrap();

    // After load: all 5 mandatory sections should be populated with
    // the exact JSON value written by setup_config_dir_at.
    let expected = serde_json::json!({"version": "1.0"});
    for section in &mandatory_sections {
        let value = manager.section(*section).unwrap();
        assert_eq!(value, expected, "section {:?} mismatch", section);
    }
}

/// Test: load() fails when one mandatory file is missing, returning
/// ConfigFileNotFound for the first missing section.
#[test]
fn test_load_fails_on_missing_mandatory_file() {
    let tmp = tempfile::tempdir().unwrap();
    // Create only models.json, missing the other 4
    fs::write(tmp.path().join("models.json"), r#"{"version": "1.0"}"#).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    let result = manager.load();
    assert!(result.is_err(), "load() should fail when files are missing");
    assert!(
        matches!(result.unwrap_err(), ConfigLoadError::ConfigFileNotFound(_)),
        "error should be ConfigFileNotFound"
    );
}

/// Test: load() fails with ConfigDirNotFound when the config directory
/// does not exist at the time of the call.
#[test]
fn test_load_fails_on_missing_config_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("vanishing_dir");
    let manager = ConfigManager::new(target.clone()).unwrap();
    // Remove the directory after construction to simulate it disappearing
    fs::remove_dir_all(&target).ok();
    let result = manager.load();
    assert!(
        result.is_err(),
        "load() should fail when config dir is gone"
    );
    assert!(
        matches!(result.unwrap_err(), ConfigLoadError::ConfigDirNotFound(_)),
        "error should be ConfigDirNotFound"
    );
}

// =====================================================================
// Step 1.5 — agent_permissions default baseline tests
// =====================================================================

/// Test: agent registered but missing permissions.json → full seven-dim Deny default.
#[test]
fn test_agent_permissions_missing_file_returns_default() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);

    // Create agents.json with one registered agent
    let agents_json = r#"{ "version": "1.0", "agents": ["test-agent"] }"#;
    let agents_dir = tmp.path().join("config");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(agents_dir.join("agents.json"), agents_json).unwrap();

    // Create agent directory with config.json but NO permissions.json
    let agent_dir = tmp.path().join("agents").join("test-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    let agent_config = r#"{ "id": "test-agent", "name": "Test Agent" }"#;
    fs::write(agent_dir.join("config.json"), agent_config).unwrap();
    // Deliberately NOT creating permissions.json

    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    // load_agents() doesn't require load() first
    manager.load_agents(None).unwrap();

    let perms = manager.agent_permissions();
    let agent_perms = perms
        .get("test-agent")
        .expect("test-agent should be in perms");

    // Should be fully denied: empty permissions map (all seven dimensions absent)
    assert!(agent_perms.is_fully_denied());
    assert_eq!(agent_perms.agent_id, "test-agent");
    assert!(agent_perms.inherited_from.is_none());

    // Verify each of the seven dimensions is denied
    for dim in &[
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
    ] {
        assert!(
            !agent_perms.is_allowed(dim),
            "dimension '{}' should be denied",
            dim
        );
    }
}
