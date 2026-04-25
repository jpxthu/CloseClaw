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
// ConfigManager — happy path
// ---------------------------------------------------------------------------

fn setup_config_dir(tmp: &tempfile::TempDir) {
    fs::write(tmp.path().join("models.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(tmp.path().join("channels.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(tmp.path().join("gateway.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(tmp.path().join("plugins.json"), r#"{"version": "1.0"}"#).unwrap();
    fs::write(tmp.path().join("system.json"), r#"{"version": "1.0"}"#).unwrap();
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
    assert!(manager.section(ConfigSection::Models).is_some());
    assert!(manager.section(ConfigSection::Channels).is_some());
    assert!(manager.section(ConfigSection::Gateway).is_some());
    assert!(manager.section(ConfigSection::Plugins).is_some());
    assert!(manager.section(ConfigSection::System).is_some());
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
