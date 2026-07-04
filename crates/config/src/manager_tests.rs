//! Tests for ConfigManager

use super::*;
use std::fs;

/// Write the 5 mandatory config files into `dir`.
/// Duplicated from common::test_helpers to avoid cross-crate test dependency.
fn write_mandatory_configs(dir: &std::path::Path) -> std::io::Result<()> {
    for name in &[
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        std::fs::write(
            dir.join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )?;
    }
    Ok(())
}

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
    assert_eq!(ConfigSection::Session.to_string(), "session.json");
    assert_eq!(ConfigSection::Credentials.to_string(), "credentials/");
    assert_eq!(ConfigSection::Accounts.to_string(), "accounts.json");
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
    write_mandatory_configs(dir).unwrap();
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

// ---------------------------------------------------------------------------
// ConfigManager — Session integration
// ---------------------------------------------------------------------------

/// Test: session_config_provider returns None before load(), Some after.
#[test]
fn test_session_config_provider_none_before_load() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    assert!(manager.session_config_provider().is_none());
}

/// Test: session_config_provider returns Some after load().
#[test]
fn test_session_config_provider_some_after_load() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();
    assert!(manager.session_config_provider().is_some());
}

/// Test: session_config_provider provides valid config when session.json exists.
#[test]
fn test_session_config_provider_with_valid_session_json() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    fs::write(
        tmp.path().join("session.json"),
        r#"{"defaults":{},"agents":{},"sweeperIntervalSecs":600}"#,
    )
    .unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let provider = manager.session_config_provider().unwrap();
    assert_eq!(provider.sweeper_interval_secs(), 600);
}

/// Test: session_config_provider uses defaults when session.json is absent.
#[test]
fn test_session_config_provider_defaults_when_no_session_json() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    // No session.json written — ConfigManager still loads successfully
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let provider = manager.session_config_provider().unwrap();
    // Should use the DEFAULT_SWEEPER_INTERVAL_SECS (300)
    assert_eq!(
        provider.sweeper_interval_secs(),
        crate::session::DEFAULT_SWEEPER_INTERVAL_SECS
    );
}

/// Test: session_config_provider uses defaults when session.json is invalid.
#[test]
fn test_session_config_provider_defaults_when_session_json_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    fs::write(tmp.path().join("session.json"), "not valid json").unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let provider = manager.session_config_provider().unwrap();
    assert_eq!(
        provider.sweeper_interval_secs(),
        crate::session::DEFAULT_SWEEPER_INTERVAL_SECS
    );
}

/// Test: list_configs includes Session section but not Credentials.
#[test]
fn test_list_configs_includes_session_excludes_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let session_path = tmp.path().join("session.json");
    fs::write(
        &session_path,
        r#"{"defaults":{},"agents":{},"sweeperIntervalSecs":600}"#,
    )
    .unwrap();
    // Verify file was written correctly
    let content = fs::read_to_string(&session_path).unwrap();
    assert!(
        !content.is_empty(),
        "session.json should not be empty after write"
    );
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let infos = manager.list_configs();
    let section_names: Vec<&str> = infos
        .iter()
        .map(|i| i.path.rsplit('/').next().unwrap_or(&i.path))
        .collect();

    assert!(
        section_names.contains(&"session.json"),
        "list_configs should include session.json, got: {:?}",
        section_names
    );
    assert!(
        !section_names.iter().any(|n| n.contains("credentials")),
        "list_configs should not include credentials, got: {:?}",
        section_names
    );
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
    // agents.json lives in config_dir (where ConfigManager reads from)
    fs::write(tmp.path().join("agents.json"), agents_json).unwrap();

    // Create agent directory with config.json but NO permissions.json
    // agents/ is at root level (parent of config_dir)
    let agent_dir = tmp
        .path()
        .parent()
        .unwrap()
        .join("agents")
        .join("test-agent");
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

// =========================================================================
// =========================================================================
// Step 1.8 — load() business validation integration tests
// =========================================================================

/// Helper: write a backup file for the given section.
/// Backup naming follows BackupManager convention: `{stem}.{timestamp}.{ext}`
fn write_backup(dir: &std::path::Path, section: ConfigSection, content: &str) {
    let backup_dir = dir.join(".backups");
    fs::create_dir_all(&backup_dir).unwrap();
    let filename = section.filename();
    let stem = filename.split('.').next().unwrap_or(filename);
    let ext = filename.split('.').last().unwrap_or("json");
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S_%f");
    let backup_name = format!("{}.{}.{}", stem, ts, ext);
    fs::write(backup_dir.join(backup_name), content).unwrap();
}

/// Test: load() triggers rollback when models.json has business validation
/// failure (empty provider ID). Verifies file is restored from backup.
#[test]
fn test_load_business_validation_failure_models_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Create a backup of the valid config, then corrupt the file
    let valid = fs::read_to_string(tmp.path().join("models.json")).unwrap();
    write_backup(tmp.path(), ConfigSection::Models, &valid);
    fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"":{"models":[]}}}"#,
    )
    .unwrap();

    // load() should succeed because rollback restores the valid backup
    manager.load().unwrap();
    // In-memory cache should be updated to the backup value
    let section = manager.section(ConfigSection::Models).unwrap();
    assert_eq!(section["version"], "1.0");
}

/// Test: load() triggers rollback when channels.json has business validation
/// failure (unknown channel type).
#[test]
fn test_load_business_validation_failure_channels_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let valid = fs::read_to_string(tmp.path().join("channels.json")).unwrap();
    write_backup(tmp.path(), ConfigSection::Channels, &valid);
    fs::write(
        tmp.path().join("channels.json"),
        r#"{"channels":{"unknown-type":{"enabled":true}}}"#,
    )
    .unwrap();

    manager.load().unwrap();
    let section = manager.section(ConfigSection::Channels).unwrap();
    assert_eq!(section["version"], "1.0");
}

/// Test: load() triggers rollback when gateway.json has business validation
/// failure (invalid port).
#[test]
fn test_load_business_validation_failure_gateway_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let valid = fs::read_to_string(tmp.path().join("gateway.json")).unwrap();
    write_backup(tmp.path(), ConfigSection::Gateway, &valid);
    fs::write(tmp.path().join("gateway.json"), r#"{"port":99999}"#).unwrap();

    manager.load().unwrap();
    let section = manager.section(ConfigSection::Gateway).unwrap();
    assert_eq!(section["version"], "1.0");
}

/// Test: load() triggers rollback when plugins.json has business validation
/// failure (empty plugin name).
#[test]
fn test_load_business_validation_failure_plugins_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let valid = fs::read_to_string(tmp.path().join("plugins.json")).unwrap();
    write_backup(tmp.path(), ConfigSection::Plugins, &valid);
    fs::write(
        tmp.path().join("plugins.json"),
        r#"{"entries":{"":{"enabled":true}}}"#,
    )
    .unwrap();

    manager.load().unwrap();
    let section = manager.section(ConfigSection::Plugins).unwrap();
    assert_eq!(section["version"], "1.0");
}

/// Test: load() triggers rollback when system.json has business validation
/// failure (empty version string).
#[test]
fn test_load_business_validation_failure_system_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let valid = fs::read_to_string(tmp.path().join("system.json")).unwrap();
    write_backup(tmp.path(), ConfigSection::System, &valid);
    fs::write(tmp.path().join("system.json"), r#"{"version":""}"#).unwrap();

    manager.load().unwrap();
    let section = manager.section(ConfigSection::System).unwrap();
    assert_eq!(section["version"], "1.0");
}

/// Test: load() succeeds normally when all mandatory sections have valid
/// business values.
#[test]
fn test_load_business_validation_success_all_sections() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    for section in &[
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ] {
        let value = manager.section(*section).unwrap();
        assert_eq!(value["version"], "1.0", "section {:?} mismatch", section);
    }
}

/// Test: load() with invalid session.json (negative idleMinutes) falls back to defaults.
#[test]
fn test_load_invalid_session_fallback_to_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    // Session with negative idleMinutes in defaults — validation fails
    fs::write(
        tmp.path().join("session.json"),
        r#"{"defaults":{"mainAgent":{"idleMinutes":-1}},"agents":{}}"#,
    )
    .unwrap();

    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let session = manager.session_config_provider().unwrap();
    assert_eq!(
        session.sweeper_interval_secs(),
        crate::session::DEFAULT_SWEEPER_INTERVAL_SECS
    );
}

/// Test: load() with valid session.json loads successfully.
#[test]
fn test_load_session_business_validation_success() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    fs::write(
        tmp.path().join("session.json"),
        r#"{"defaults":{},"agents":{},"sweeperIntervalSecs":900}"#,
    )
    .unwrap();

    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    let session = manager.session_config_provider().unwrap();
    assert_eq!(session.sweeper_interval_secs(), 900);
}

/// Test: load() with multiple invalid sections triggers rollback for each.
#[test]
fn test_load_multiple_business_validation_failures_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();

    // Create backups for both sections before corruption
    let valid_models = fs::read_to_string(tmp.path().join("models.json")).unwrap();
    write_backup(tmp.path(), ConfigSection::Models, &valid_models);
    let valid_gw = fs::read_to_string(tmp.path().join("gateway.json")).unwrap();
    write_backup(tmp.path(), ConfigSection::Gateway, &valid_gw);

    // Corrupt two mandatory sections simultaneously
    fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"":{"models":[]}}}"#,
    )
    .unwrap();
    fs::write(tmp.path().join("gateway.json"), r#"{"port":99999}"#).unwrap();

    // load() should succeed: both sections rollback independently
    manager.load().unwrap();
    assert_eq!(
        manager.section(ConfigSection::Models).unwrap()["version"],
        "1.0"
    );
    assert_eq!(
        manager.section(ConfigSection::Gateway).unwrap()["version"],
        "1.0"
    );
}

/// Test: load() with empty models array (no providers key) passes validation.
#[test]
fn test_load_models_empty_providers_passes() {
    let tmp = tempfile::tempdir().unwrap();
    write_mandatory_configs(tmp.path()).unwrap();
    fs::write(tmp.path().join("models.json"), r#"{"providers":{}}"#).unwrap();

    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();
    assert_eq!(
        manager.section(ConfigSection::Models).unwrap()["providers"],
        serde_json::json!({})
    );
}
