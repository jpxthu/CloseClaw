use super::*;

fn setup_config_dir(tmp: &tempfile::TempDir) {
    for name in &[
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        std::fs::write(
            tmp.path().join(name),
            serde_json::json!({"version": "1.0"}).to_string(),
        )
        .unwrap();
    }
}

// ---------------------------------------------------------------------------
// ConfigSection::is_restart_class()
// ---------------------------------------------------------------------------

#[test]
fn test_is_restart_class_true_for_restart_sections() {
    assert!(ConfigSection::Models.is_restart_class());
    assert!(ConfigSection::Channels.is_restart_class());
    assert!(ConfigSection::Gateway.is_restart_class());
    assert!(ConfigSection::Credentials.is_restart_class());
}

#[test]
fn test_is_restart_class_false_for_non_restart_sections() {
    assert!(!ConfigSection::Plugins.is_restart_class());
    assert!(!ConfigSection::System.is_restart_class());
    assert!(!ConfigSection::Session.is_restart_class());
    assert!(!ConfigSection::Accounts.is_restart_class());
    assert!(!ConfigSection::Memory.is_restart_class());
    assert!(!ConfigSection::Skills.is_restart_class());
}

// ---------------------------------------------------------------------------
// stage_restart_value / apply_pending_restart / pending_restart_value
// ---------------------------------------------------------------------------

#[test]
fn test_stage_restart_value_writes_pending_not_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();
    assert_eq!(
        manager.section(ConfigSection::Models).unwrap()["version"],
        "1.0"
    );
    let staged = serde_json::json!({"version": "9.9"});
    let path = ConfigSection::Models.path(tmp.path());
    manager.stage_restart_value(ConfigSection::Models, path, staged.clone());
    assert_eq!(
        manager
            .pending_restart_value(ConfigSection::Models)
            .unwrap(),
        staged
    );
    assert_eq!(
        manager.section(ConfigSection::Models).unwrap()["version"],
        "1.0"
    );
}

#[test]
fn test_apply_pending_restart_moves_to_runtime_and_clears() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();
    manager.stage_restart_value(
        ConfigSection::Models,
        ConfigSection::Models.path(tmp.path()),
        serde_json::json!({"version": "9.9"}),
    );
    manager.stage_restart_value(
        ConfigSection::Gateway,
        ConfigSection::Gateway.path(tmp.path()),
        serde_json::json!({"port": 12345}),
    );
    assert_eq!(
        manager.section(ConfigSection::Models).unwrap()["version"],
        "1.0"
    );
    manager.apply_pending_restart();
    assert_eq!(
        manager.section(ConfigSection::Models).unwrap()["version"],
        "9.9"
    );
    assert_eq!(
        manager.section(ConfigSection::Gateway).unwrap()["port"],
        12345
    );
    assert!(manager
        .pending_restart_value(ConfigSection::Models)
        .is_none());
    assert!(manager
        .pending_restart_value(ConfigSection::Gateway)
        .is_none());
}

#[test]
fn test_apply_pending_restart_noop_when_empty() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();
    let old = manager.section(ConfigSection::Models).unwrap();
    manager.apply_pending_restart();
    assert_eq!(manager.section(ConfigSection::Models).unwrap(), old);
}

#[test]
fn test_pending_restart_value_none_for_unstaged() {
    let tmp = tempfile::tempdir().unwrap();
    setup_config_dir(&tmp);
    let manager = ConfigManager::new(tmp.path().to_path_buf()).unwrap();
    manager.load().unwrap();
    assert!(manager
        .pending_restart_value(ConfigSection::Models)
        .is_none());
    assert!(manager
        .pending_restart_value(ConfigSection::Gateway)
        .is_none());
    assert!(manager
        .pending_restart_value(ConfigSection::Channels)
        .is_none());
}
