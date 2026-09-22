//! Tests for daemon-level reload helpers: agent-id extraction from
//! permissions paths and `DaemonReloadCallback` reload reactions.
//!
//! Layout mirrors production: config files under `<root>/config/`,
//! agent directories under `<root>/agents/`, both inside the TempDir.

use crate::config_reload::reload::{extract_agent_id_from_permissions_path, DaemonReloadCallback};
use crate::test_helpers::write_mandatory_configs;
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_config::agents::AgentPermissionProvider;
use closeclaw_config::manager::ConfigManager;
use closeclaw_config::ReloadCallback;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

/// The config subdir under a temp root (`<root>/config`).
fn config_dir_under(root: &Path) -> PathBuf {
    root.join("config")
}

/// Build a ConfigManager over `<root>/config` with the mandatory
/// skeleton configs written, so agent directories resolve to
/// `<root>/agents` (production layout: `config_dir.parent()/agents`).
fn make_config_manager(root: &Path) -> Arc<ConfigManager> {
    let config_dir = config_dir_under(root);
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    write_mandatory_configs(&config_dir).expect("mandatory configs");
    let cm = ConfigManager::new(config_dir).expect("ConfigManager::new");
    cm.load().expect("ConfigManager::load");
    Arc::new(cm)
}

fn make_agent_registry() -> Arc<AgentRegistry> {
    Arc::new(AgentRegistry::new())
}

/// Create `<root>/agents/<id>/` containing a `config.json`.
fn make_agent_dir(root: &Path, id: &str, name: &str) -> PathBuf {
    let dir = root.join("agents").join(id);
    std::fs::create_dir_all(&dir).expect("create agent dir");
    let json = format!(r#"{{"id":"{id}","name":"{name}"}}"#);
    std::fs::write(dir.join("config.json"), json).expect("write agent config");
    dir
}

// ------------------------------------------------------------------
// extract_agent_id_from_permissions_path
// ------------------------------------------------------------------

#[test]
fn test_extract_agent_id_from_permissions_path() {
    let d = TempDir::new().unwrap();
    let agents_dir = d.path().join("agents").join("gamma");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("config.json"), r#"{"id":"gamma"}"#).unwrap();
    let perm_path = agents_dir.join("permissions.json");
    assert_eq!(
        extract_agent_id_from_permissions_path(&perm_path),
        Some("gamma".to_string())
    );
}

#[test]
fn test_extract_agent_id_no_config_json() {
    let d = TempDir::new().unwrap();
    let agents_dir = d.path().join("agents").join("ghost");
    std::fs::create_dir_all(&agents_dir).unwrap();
    let perm_path = agents_dir.join("permissions.json");
    assert_eq!(extract_agent_id_from_permissions_path(&perm_path), None);
}

// ------------------------------------------------------------------
// DaemonReloadCallback — agent reload + registry sync
// ------------------------------------------------------------------

#[test]
fn test_daemon_callback_agents_changed_syncs_registry() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let ar = make_agent_registry();
    let callback = DaemonReloadCallback::new_for_test(ar.clone());

    let agents_json_path = config_dir_under(d.path()).join("agents.json");
    std::fs::write(&agents_json_path, r#"{ "agents": ["alpha"] }"#).unwrap();
    make_agent_dir(d.path(), "alpha", "Alpha");

    callback.on_agents_changed(&agents_json_path, &cm);

    let agents: Vec<_> = ar.iter().map(|e| e.key().clone()).collect();
    assert!(
        agents.contains(&"alpha".to_string()),
        "AgentRegistry should contain alpha after callback; registry keys: {agents:?}"
    );
}

#[test]
fn test_daemon_callback_agents_failure_no_disk_rollback() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let ar = make_agent_registry();
    let callback = DaemonReloadCallback::new_for_test(ar);

    let agents_json_path = config_dir_under(d.path()).join("agents.json");
    std::fs::write(&agents_json_path, r#"{ "agents": ["alpha"] }"#).unwrap();
    let alpha_dir = make_agent_dir(d.path(), "alpha", "Alpha");

    cm.load_agents(None).unwrap();
    let old_agents = cm.snapshot_agents();

    // Backup before modification
    let _ = cm.backup_manager().backup(&agents_json_path);
    let _ = cm.backup_manager().backup(alpha_dir.join("config.json"));

    let original_agents_json = std::fs::read_to_string(&agents_json_path).unwrap();

    // Add beta, reload through the daemon callback
    make_agent_dir(d.path(), "beta", "Beta");
    std::fs::write(&agents_json_path, r#"{ "agents": ["alpha", "beta"] }"#).unwrap();
    callback.on_agents_changed(&agents_json_path, &cm);
    assert!(
        cm.agents().contains_key("beta"),
        "reload should pick up beta from disk; in-memory agents: {:?}",
        cm.agents().keys().collect::<Vec<_>>()
    );

    cm.restore_agents(old_agents);

    assert!(
        cm.agents().contains_key("alpha"),
        "restore_agents should keep the pre-reload alpha entry"
    );
    assert!(
        !cm.agents().contains_key("beta"),
        "restore_agents should drop the beta entry added by reload"
    );

    // Disk NOT rolled back
    let current = std::fs::read_to_string(&agents_json_path).unwrap();
    assert_ne!(
        current, original_agents_json,
        "restore_agents is in-memory only; agents.json on disk should still list beta"
    );
}

// ------------------------------------------------------------------
// DaemonReloadCallback — permissions
// ------------------------------------------------------------------

#[test]
fn test_daemon_callback_permissions_changed() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let ar = make_agent_registry();
    let callback = DaemonReloadCallback::new_for_test(ar);

    let epsilon_dir = make_agent_dir(d.path(), "epsilon", "Epsilon");
    let perms_path = epsilon_dir.join("permissions.json");
    std::fs::write(&perms_path, r#"{"agent_id":"epsilon","permissions":{}}"#).unwrap();

    let agents_json = config_dir_under(d.path()).join("agents.json");
    std::fs::write(&agents_json, r#"{"agents":["epsilon"]}"#).unwrap();
    cm.load_agents(None).unwrap();

    let before = cm.agent_permissions();
    assert!(
        before.get("epsilon").is_some(),
        "epsilon permissions should load from disk before the invalid write"
    );

    // Write invalid JSON
    std::fs::write(&perms_path, "not valid json{{").unwrap();

    callback.on_permissions_changed(&perms_path, &cm);

    let after = cm.agent_permissions();
    assert!(
        after.get("epsilon").is_none(),
        "lazy loader should return None for invalid permissions file"
    );
}

// ------------------------------------------------------------------
// DaemonReloadCallback — session
// ------------------------------------------------------------------

#[test]
fn test_daemon_callback_session_reloaded() {
    let d = TempDir::new().unwrap();
    let config_dir = config_dir_under(d.path());
    std::fs::create_dir_all(&config_dir).unwrap();
    write_mandatory_configs(&config_dir).unwrap();
    let session_path = config_dir.join("session.json");
    std::fs::write(
        &session_path,
        r#"{"defaults":{},"agents":{},"sweeperIntervalSeconds":600}"#,
    )
    .unwrap();
    let cm = Arc::new(ConfigManager::new(config_dir).unwrap());
    cm.load().unwrap();
    let ar = make_agent_registry();
    let callback = DaemonReloadCallback::new_for_test(ar);

    let provider = cm.session_config_provider().unwrap();
    assert_eq!(provider.sweeper_interval_secs(), 600);

    std::fs::write(
        &session_path,
        r#"{"defaults":{},"agents":{},"sweeperIntervalSeconds":9999}"#,
    )
    .unwrap();

    callback.on_session_reloaded(&cm);

    let provider = cm.session_config_provider().unwrap();
    assert_eq!(provider.sweeper_interval_secs(), 9999);
}

// ------------------------------------------------------------------
// ConfigReloadManager import from config crate
// ------------------------------------------------------------------

#[test]
fn test_config_reload_manager_importable_from_config_crate() {
    let d = TempDir::new().unwrap();
    let cm = make_config_manager(d.path());
    let ar = make_agent_registry();
    let callback = Arc::new(DaemonReloadCallback::new_for_test(ar));
    let _mgr = closeclaw_config::ConfigReloadManager::with_defaults(cm, callback);
}
