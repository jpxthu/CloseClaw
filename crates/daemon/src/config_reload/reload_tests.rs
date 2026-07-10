#[cfg(test)]
mod reload_tests {
    use crate::config_reload::reload::{
        extract_agent_id_from_permissions_path, DaemonReloadCallback,
    };
    use closeclaw_agent::registry::AgentRegistry;
    use closeclaw_config::manager::{ConfigManager, ConfigSection};
    use closeclaw_config::ReloadCallback;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_config_manager(dir: &std::path::Path) -> Arc<ConfigManager> {
        let sections = [
            ("models.json", r#"{"models":[]}"#),
            ("channels.json", r#"{"channels":{}}"#),
            ("gateway.json", r#"{"port":8080}"#),
            ("plugins.json", r#"{"plugins":[]}"#),
            ("system.json", r#"{"version":"1"}"#),
            ("accounts.json", r#"{"accounts":[]}"#),
        ];
        for (name, content) in &sections {
            std::fs::write(dir.join(name), content).unwrap();
        }
        let cm = ConfigManager::new(dir.to_path_buf()).unwrap();
        cm.load().unwrap();
        Arc::new(cm)
    }

    fn make_agent_registry() -> Arc<AgentRegistry> {
        Arc::new(AgentRegistry::new())
    }

    // ------------------------------------------------------------------
    // extract_agent_id_from_permissions_path
    // ------------------------------------------------------------------

    #[test]
    fn test_extract_agent_id_from_permissions_path() {
        let d = TempDir::new().unwrap();
        let root_dir = d.path().parent().unwrap().to_path_buf();
        let agents_dir = root_dir.join("agents").join("gamma");
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
        let callback = DaemonReloadCallback::new(ar.clone());

        let agents_json_path = d.path().join("agents.json");
        std::fs::write(&agents_json_path, r#"{ "agents": ["alpha"] }"#).unwrap();

        let root_dir = d.path().parent().unwrap().to_path_buf();
        let alpha_dir = root_dir.join("agents").join("alpha");
        std::fs::create_dir_all(&alpha_dir).unwrap();
        std::fs::write(
            alpha_dir.join("config.json"),
            r#"{ "id": "alpha", "name": "Alpha" }"#,
        )
        .unwrap();

        callback.on_agents_changed(&agents_json_path, &cm);

        let agents: Vec<_> = ar.iter().map(|e| e.key().clone()).collect();
        assert!(
            agents.contains(&"alpha".to_string()),
            "AgentRegistry should contain alpha after callback"
        );
    }

    #[test]
    fn test_daemon_callback_agents_failure_no_disk_rollback() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let callback = DaemonReloadCallback::new(ar);

        let agents_json_path = d.path().join("agents.json");
        std::fs::write(&agents_json_path, r#"{ "agents": ["alpha"] }"#).unwrap();

        let root_dir = d.path().parent().unwrap().to_path_buf();
        let alpha_dir = root_dir.join("agents").join("alpha");
        std::fs::create_dir_all(&alpha_dir).unwrap();
        std::fs::write(
            alpha_dir.join("config.json"),
            r#"{ "id": "alpha", "name": "Alpha" }"#,
        )
        .unwrap();

        cm.load_agents(None).unwrap();
        let old_agents = cm.snapshot_agents();

        // Backup before modification
        let _ = cm.backup_manager().backup(&agents_json_path);
        let _ = cm.backup_manager().backup(alpha_dir.join("config.json"));

        let original_agents_json = std::fs::read_to_string(&agents_json_path).unwrap();

        // Add beta
        let beta_dir = root_dir.join("agents").join("beta");
        std::fs::create_dir_all(&beta_dir).unwrap();
        std::fs::write(
            beta_dir.join("config.json"),
            r#"{ "id": "beta", "name": "Beta" }"#,
        )
        .unwrap();
        std::fs::write(&agents_json_path, r#"{ "agents": ["alpha", "beta"] }"#).unwrap();
        cm.reload_agents().unwrap();
        assert!(cm.agents().contains_key("beta"));

        cm.restore_agents(old_agents);

        assert!(cm.agents().contains_key("alpha"));
        assert!(!cm.agents().contains_key("beta"));

        // Disk NOT rolled back
        let current = std::fs::read_to_string(&agents_json_path).unwrap();
        assert_ne!(current, original_agents_json);
    }

    // ------------------------------------------------------------------
    // DaemonReloadCallback — permissions
    // ------------------------------------------------------------------

    #[test]
    fn test_daemon_callback_permissions_changed() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let callback = DaemonReloadCallback::new(ar);

        let root_dir = d.path().parent().unwrap().to_path_buf();
        let agents_dir = root_dir.join("agents").join("epsilon");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(
            agents_dir.join("config.json"),
            r#"{"id":"epsilon","name":"Epsilon"}"#,
        )
        .unwrap();
        let perms_path = agents_dir.join("permissions.json");
        std::fs::write(&perms_path, r#"{"agent_id":"epsilon","permissions":{}}"#).unwrap();

        let agents_json = d.path().join("agents.json");
        std::fs::write(&agents_json, r#"{"agents":["epsilon"]}"#).unwrap();
        cm.load_agents(None).unwrap();

        let before = cm.agent_permissions();
        assert!(before.get("epsilon").is_some());

        // Sleep to ensure mtime changes
        std::thread::sleep(std::time::Duration::from_secs(1));

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
        let sections = [
            ("models.json", r#"{"models":[]}"#),
            ("channels.json", r#"{"channels":{}}"#),
            ("gateway.json", r#"{"port":8080}"#),
            ("plugins.json", r#"{"plugins":[]}"#),
            ("system.json", r#"{"version":"1"}"#),
            ("accounts.json", r#"{"accounts":[]}"#),
            (
                "session.json",
                r#"{"defaults":{},"agents":{},"sweeperIntervalSecs":600}"#,
            ),
        ];
        for (name, content) in &sections {
            std::fs::write(d.path().join(name), content).unwrap();
        }
        let cm = Arc::new(ConfigManager::new(d.path().to_path_buf()).unwrap());
        cm.load().unwrap();
        let ar = make_agent_registry();
        let callback = DaemonReloadCallback::new(ar);

        let provider = cm.session_config_provider().unwrap();
        assert_eq!(provider.sweeper_interval_secs(), 600);

        std::fs::write(
            d.path().join("session.json"),
            r#"{"defaults":{},"agents":{},"sweeperIntervalSecs":9999}"#,
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
        let callback = Arc::new(DaemonReloadCallback::new(ar));
        let _mgr = closeclaw_config::ConfigReloadManager::with_defaults(cm, callback);
    }
}
