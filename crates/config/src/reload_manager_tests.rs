#[cfg(test)]
mod tests {
    use crate::manager::{ConfigManager, ConfigSection};
    use crate::reload_manager::{
        dispatch_change, filename_to_section, is_agents_path, is_permissions_path,
        ConfigReloadManager, ReloadCallback, DEFAULT_DEBOUNCE,
    };
    use std::path::Path;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;
    use tempfile::TempDir;

    /// Mock callback that records reload invocations.
    struct MockCallback {
        agents_called: std::sync::atomic::AtomicBool,
        permissions_called: std::sync::atomic::AtomicBool,
        session_called: std::sync::atomic::AtomicBool,
    }

    impl MockCallback {
        fn new() -> Self {
            Self {
                agents_called: std::sync::atomic::AtomicBool::new(false),
                permissions_called: std::sync::atomic::AtomicBool::new(false),
                session_called: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn was_agents_called(&self) -> bool {
            self.agents_called
                .load(std::sync::atomic::Ordering::Relaxed)
        }

        fn was_permissions_called(&self) -> bool {
            self.permissions_called
                .load(std::sync::atomic::Ordering::Relaxed)
        }

        fn was_session_called(&self) -> bool {
            self.session_called
                .load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl ReloadCallback for MockCallback {
        fn on_agents_changed(&self, _path: &Path, _cm: &ConfigManager) {
            self.agents_called
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        fn on_permissions_changed(&self, _path: &Path, _cm: &ConfigManager) {
            self.permissions_called
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        fn on_session_reloaded(&self, _cm: &ConfigManager) {
            self.session_called
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn make_config_manager(dir: &std::path::Path) -> Arc<ConfigManager> {
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
            std::fs::write(dir.join(name), content).unwrap();
        }
        let cm = ConfigManager::new(dir.to_path_buf()).unwrap();
        cm.load().unwrap();
        Arc::new(cm)
    }

    // ------------------------------------------------------------------
    // Basic creation tests
    // ------------------------------------------------------------------

    #[test]
    fn test_new_and_defaults() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm, cb);
        assert_eq!(mgr.debounce_duration, DEFAULT_DEBOUNCE);
    }

    #[test]
    fn test_new_custom_debounce() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::new(cm, cb, Duration::from_secs(2));
        assert_eq!(mgr.debounce_duration, Duration::from_secs(2));
    }

    #[test]
    fn test_watch_returns_handle() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mut mgr = ConfigReloadManager::with_defaults(cm, cb);
        let handle = mgr.watch(d.path().to_str().unwrap());
        assert!(handle.is_ok());
        drop(handle.unwrap());
    }

    // ------------------------------------------------------------------
    // Filename to section mapping
    // ------------------------------------------------------------------

    #[test]
    fn test_filename_to_section_mapping() {
        assert_eq!(
            filename_to_section("models.json"),
            Some(ConfigSection::Models)
        );
        assert_eq!(
            filename_to_section("channels.json"),
            Some(ConfigSection::Channels)
        );
        assert_eq!(
            filename_to_section("gateway.json"),
            Some(ConfigSection::Gateway)
        );
        assert_eq!(
            filename_to_section("plugins.json"),
            Some(ConfigSection::Plugins)
        );
        assert_eq!(
            filename_to_section("system.json"),
            Some(ConfigSection::System)
        );
        assert_eq!(
            filename_to_section("session.json"),
            Some(ConfigSection::Session)
        );
        assert_eq!(
            filename_to_section("accounts.json"),
            Some(ConfigSection::Accounts)
        );
        assert_eq!(
            filename_to_section("memory.json"),
            Some(ConfigSection::Memory)
        );
        assert_eq!(filename_to_section("agents.json"), None);
        assert_eq!(filename_to_section("unknown.json"), None);
    }

    // ------------------------------------------------------------------
    // Path classification helpers
    // ------------------------------------------------------------------

    #[test]
    fn test_is_agents_path() {
        assert!(is_agents_path(Path::new("/config/agents/test.json")));
        assert!(is_agents_path(Path::new("/config\\agents\\test.json")));
        assert!(is_agents_path(Path::new(
            "/config/agents/alpha/permissions.json"
        )));
        assert!(!is_agents_path(Path::new("/config/models.json")));
    }

    #[test]
    fn test_is_permissions_path() {
        assert!(is_permissions_path(Path::new(
            "/config/agents/alpha/permissions.json"
        )));
        assert!(!is_permissions_path(Path::new(
            "/config/agents/alpha/config.json"
        )));
        assert!(!is_permissions_path(Path::new("/config/models.json")));
    }

    // ------------------------------------------------------------------
    // Dispatch tests — callback invocation
    // ------------------------------------------------------------------

    #[test]
    fn test_dispatch_section_change() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        std::fs::write(d.path().join("models.json"), r#"{"models":[{"id":"m1"}]}"#).unwrap();

        let path = d.path().join("models.json");
        dispatch_change(&path, &mgr);

        let val = cm.section(ConfigSection::Models).unwrap();
        assert!(val.get("models").is_some());
    }

    #[test]
    fn test_dispatch_agents_json_invokes_callback() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm, cb.clone());

        std::fs::create_dir_all(d.path().join("agents")).unwrap();
        std::fs::write(d.path().join("agents.json"), r#"{"agents":{}}"#).unwrap();

        let path = d.path().join("agents.json");
        dispatch_change(&path, &mgr);

        assert!(
            cb.was_agents_called(),
            "agents callback should be invoked for agents.json"
        );
    }

    #[test]
    fn test_dispatch_unknown_file_is_noop() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm, cb.clone());

        let path = d.path().join("unknown.json");
        dispatch_change(&path, &mgr);

        assert!(
            !cb.was_agents_called(),
            "agents callback should NOT be invoked for unknown files"
        );
        assert!(
            !cb.was_permissions_called(),
            "permissions callback should NOT be invoked for unknown files"
        );
    }

    #[test]
    fn test_dispatch_agents_dir_change_invokes_callback() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm, cb.clone());

        let agents_dir = d.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("test.json"), r#"{}"#).unwrap();

        let path = agents_dir.join("test.json");
        dispatch_change(&path, &mgr);

        assert!(
            cb.was_agents_called(),
            "agents callback should be invoked for agents/ dir changes"
        );
    }

    #[test]
    fn test_dispatch_permissions_invokes_permissions_callback() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm, cb.clone());

        let root_dir = d.path().parent().unwrap().to_path_buf();
        let agents_dir = root_dir.join("agents").join("delta");
        std::fs::create_dir_all(&agents_dir).unwrap();

        let perms_path = agents_dir.join("permissions.json");
        dispatch_change(&perms_path, &mgr);

        assert!(
            cb.was_permissions_called(),
            "permissions callback should be invoked for permissions.json"
        );
        assert!(
            !cb.was_agents_called(),
            "agents callback should NOT be invoked for permissions.json"
        );
    }

    #[test]
    fn test_dispatch_session_invokes_session_callback() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm, cb.clone());

        std::fs::write(
            d.path().join("session.json"),
            r#"{"defaults":{},"agents":{},"sweeperIntervalSecs":1200}"#,
        )
        .unwrap();

        let path = d.path().join("session.json");
        dispatch_change(&path, &mgr);

        assert!(
            cb.was_session_called(),
            "session callback should be invoked after successful session reload"
        );
    }

    // ------------------------------------------------------------------
    // reload_section tests
    // ------------------------------------------------------------------

    #[test]
    fn test_reload_section_success() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        std::fs::write(d.path().join("system.json"), r#"{"version":"2.0"}"#).unwrap();
        mgr.reload_section(ConfigSection::System).unwrap();

        let val = cm.section(ConfigSection::System).unwrap();
        assert_eq!(val["version"], "2.0");
    }

    #[test]
    fn test_reload_section_validation_failure() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        std::fs::write(d.path().join("models.json"), r#"{"models":"not array"}"#).unwrap();
        let result = mgr.reload_section(ConfigSection::Models);
        assert!(result.is_err());

        // In-memory should still be the old value
        let val = cm.section(ConfigSection::Models).unwrap();
        assert!(val.get("models").is_some());
    }

    // ------------------------------------------------------------------
    // WatcherHandle RAII
    // ------------------------------------------------------------------

    #[test]
    fn test_watcher_handle_stop() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mut mgr = ConfigReloadManager::with_defaults(cm, cb);
        let handle = mgr.watch(d.path().to_str().unwrap()).unwrap();
        handle.stop();
    }

    #[test]
    fn test_watcher_handle_drop_is_safe() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mut mgr = ConfigReloadManager::with_defaults(cm, cb);
        let handle = mgr.watch(d.path().to_str().unwrap()).unwrap();
        drop(handle);
    }

    // ------------------------------------------------------------------
    // Debounce integration tests
    // ------------------------------------------------------------------

    #[test]
    fn test_rapid_file_changes_stable() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mut mgr = ConfigReloadManager::new(cm.clone(), cb, Duration::from_millis(50));
        let (tx, rx) = mpsc::channel();
        mgr.set_test_completion(tx);
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        for i in 1..=10 {
            std::fs::write(
                d.path().join("gateway.json"),
                format!(r#"{{"port":{}}}"#, 8000 + i),
            )
            .unwrap();
            std::fs::write(
                d.path().join("models.json"),
                format!(r#"{{"models":[{{"id":"m{}"}}]}}"#, i),
            )
            .unwrap();
        }

        rx.recv_timeout(Duration::from_secs(5))
            .expect("reload loop should have completed at least one dispatch cycle");

        let gw = cm.section(ConfigSection::Gateway).unwrap();
        assert!(gw.is_object(), "gateway section should be an object");
        let md = cm.section(ConfigSection::Models).unwrap();
        assert!(md.is_object(), "models section should be an object");
    }

    #[test]
    fn test_single_change_reloads_correctly() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mut mgr = ConfigReloadManager::new(cm.clone(), cb, Duration::from_millis(50));
        let (tx, rx) = mpsc::channel();
        mgr.set_test_completion(tx);
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        std::fs::write(
            d.path().join("plugins.json"),
            r#"{"plugins":[{"id":"p1"}]}"#,
        )
        .unwrap();

        rx.recv_timeout(Duration::from_secs(5))
            .expect("reload loop should have processed the file change");

        let val = cm.section(ConfigSection::Plugins).unwrap();
        assert!(val.get("plugins").is_some());
    }

    #[test]
    fn test_debounce_merges_multiple_file_changes() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mut mgr = ConfigReloadManager::new(cm.clone(), cb, Duration::from_millis(100));
        let (tx, rx) = mpsc::channel();
        mgr.set_test_completion(tx);
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        std::fs::write(d.path().join("models.json"), r#"{"models":[{"id":"m1"}]}"#).unwrap();
        std::fs::write(
            d.path().join("channels.json"),
            r#"{"channels":{"type":{}}}"#,
        )
        .unwrap();
        std::fs::write(d.path().join("gateway.json"), r#"{"port":9999}"#).unwrap();

        rx.recv_timeout(Duration::from_secs(5))
            .expect("reload loop should have dispatched all merged changes");

        let md = cm.section(ConfigSection::Models).unwrap();
        assert!(md.get("models").is_some(), "models should be reloaded");
        let ch = cm.section(ConfigSection::Channels).unwrap();
        assert!(ch.get("channels").is_some(), "channels should be reloaded");
        let gw = cm.section(ConfigSection::Gateway).unwrap();
        assert!(gw.is_object(), "gateway should be reloaded");
    }

    #[test]
    fn test_debounce_dedup_same_file() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mut mgr = ConfigReloadManager::new(cm.clone(), cb, Duration::from_millis(100));
        let (tx, rx) = mpsc::channel();
        mgr.set_test_completion(tx);
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        for i in 1..=20 {
            std::fs::write(
                d.path().join("gateway.json"),
                format!(r#"{{"port":{}}}"#, 8000 + i),
            )
            .unwrap();
        }

        rx.recv_timeout(Duration::from_secs(5))
            .expect("reload loop should have completed at least one dispatch cycle");

        let gw = cm.section(ConfigSection::Gateway).unwrap();
        assert_eq!(
            gw["port"], 8020,
            "gateway port should reflect the last write after dedup"
        );
    }

    // ------------------------------------------------------------------
    // Arc sharing tests
    // ------------------------------------------------------------------

    #[test]
    fn test_config_reload_manager_clones_arcs() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let _mgr1 = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());
        let _mgr2 = ConfigReloadManager::with_defaults(cm.clone(), cb.clone());
        let s1 = cm.section(ConfigSection::System).unwrap();
        let s2 = cm.section(ConfigSection::System).unwrap();
        assert_eq!(s1, s2, "shared Arc should produce identical data");
    }

    // ------------------------------------------------------------------
    // Dispatch all config sections
    // ------------------------------------------------------------------

    #[test]
    fn test_dispatch_all_config_sections() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        let cases = [
            (
                "channels.json",
                ConfigSection::Channels,
                r#"{"channels":{"type":{}}}"#,
            ),
            ("gateway.json", ConfigSection::Gateway, r#"{"port":9090}"#),
            (
                "plugins.json",
                ConfigSection::Plugins,
                r#"{"plugins":[{"id":"pl1"}]}"#,
            ),
            ("system.json", ConfigSection::System, r#"{"version":"2.0"}"#),
            (
                "models.json",
                ConfigSection::Models,
                r#"{"models":[{"id":"m1"}]}"#,
            ),
            (
                "session.json",
                ConfigSection::Session,
                r#"{"defaults":{},"agents":{},"sweeperIntervalSecs":999}"#,
            ),
            (
                "memory.json",
                ConfigSection::Memory,
                r#"{"mining":{"enabled":true}}"#,
            ),
        ];

        for (filename, section, content) in cases {
            std::fs::write(d.path().join(filename), content).unwrap();
            let path = d.path().join(filename);
            dispatch_change(&path, &mgr);
            let val = cm.section(section).unwrap();
            assert!(val.is_object(), "section {:?} should be an object", section);
        }
    }

    // ------------------------------------------------------------------
    // No rollback on failure
    // ------------------------------------------------------------------

    #[test]
    fn test_reload_section_parse_failure_no_rollback() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm, cb);

        std::fs::write(d.path().join("system.json"), "not json!!!").unwrap();
        let result = mgr.reload_section(ConfigSection::System);
        assert!(result.is_err());

        let content = std::fs::read_to_string(d.path().join("system.json")).unwrap();
        assert_eq!(
            content, "not json!!!",
            "file must NOT be rolled back on parse failure"
        );
    }

    #[test]
    fn test_reload_section_validation_failure_no_rollback() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm, cb);

        std::fs::write(d.path().join("models.json"), r#"{"models":"not array"}"#).unwrap();
        let result = mgr.reload_section(ConfigSection::Models);
        assert!(result.is_err());

        let content = std::fs::read_to_string(d.path().join("models.json")).unwrap();
        assert!(
            content.contains("not array"),
            "file must NOT be rolled back on validation failure"
        );
    }

    // ------------------------------------------------------------------
    // Blocking on no old value
    // ------------------------------------------------------------------

    #[test]
    fn test_reload_section_no_old_value_blocks_section() {
        let d = TempDir::new().unwrap();
        let cm = Arc::new(ConfigManager::new(d.path().to_path_buf()).unwrap());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        std::fs::write(d.path().join("system.json"), "not json").unwrap();
        assert!(mgr.reload_section(ConfigSection::System).is_err());

        assert!(cm.is_blocked(ConfigSection::System));
        assert!(cm.get_section_value(ConfigSection::System).is_none());
    }

    #[test]
    fn test_reload_section_blocked_section_cleared_on_success() {
        let d = TempDir::new().unwrap();
        let cm = Arc::new(ConfigManager::new(d.path().to_path_buf()).unwrap());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        cm.block_section(ConfigSection::System);
        assert!(cm.is_blocked(ConfigSection::System));

        std::fs::write(d.path().join("system.json"), r#"{"version":"9.9"}"#).unwrap();
        mgr.reload_section(ConfigSection::System).unwrap();

        assert!(!cm.is_blocked(ConfigSection::System));
        let val = cm.get_section_value(ConfigSection::System).unwrap();
        assert_eq!(val["version"], "9.9");
    }

    // ------------------------------------------------------------------
    // Snapshot broadcast
    // ------------------------------------------------------------------

    #[test]
    fn test_reload_section_broadcasts_snapshot() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        let mut snapshot_rx = cm.subscribe_config_snapshots();

        std::fs::write(d.path().join("system.json"), r#"{"version":"8.8"}"#).unwrap();
        mgr.reload_section(ConfigSection::System).unwrap();

        let val = cm.section(ConfigSection::System).unwrap();
        assert_eq!(val["version"], "8.8");

        let snapshot = snapshot_rx
            .try_recv()
            .expect("should receive a snapshot after successful reload");
        assert_eq!(
            snapshot.get(&ConfigSection::System).unwrap()["version"],
            "8.8"
        );
    }

    // ------------------------------------------------------------------
    // Step 1.2 — reload_section backup ordering tests
    // ------------------------------------------------------------------

    /// Validation failure should NOT create any backup file.
    #[test]
    fn test_reload_section_validation_failure_no_backup_file() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        let backups_before = cm
            .backup_manager()
            .list_backups(d.path().join("system.json"))
            .unwrap();
        let count_before = backups_before.len();

        std::fs::write(d.path().join("system.json"), r#"{"version":123}"#).unwrap();
        let result = mgr.reload_section(ConfigSection::System);
        assert!(result.is_err());

        let backups_after = cm
            .backup_manager()
            .list_backups(d.path().join("system.json"))
            .unwrap();
        assert_eq!(
            backups_after.len(),
            count_before,
            "validation failure should not create any backup file"
        );
    }

    /// Parse failure should NOT create any backup file.
    #[test]
    fn test_reload_section_parse_failure_no_backup_file() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        let backups_before = cm
            .backup_manager()
            .list_backups(d.path().join("system.json"))
            .unwrap();
        let count_before = backups_before.len();

        std::fs::write(d.path().join("system.json"), "not json!!!").unwrap();
        let result = mgr.reload_section(ConfigSection::System);
        assert!(result.is_err());

        let backups_after = cm
            .backup_manager()
            .list_backups(d.path().join("system.json"))
            .unwrap();
        assert_eq!(
            backups_after.len(),
            count_before,
            "parse failure should not create any backup file"
        );
    }

    /// Validation passes → backup file is created for old in-memory value.
    #[test]
    fn test_reload_section_success_creates_backup() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        let backups_before = cm
            .backup_manager()
            .list_backups(d.path().join("system.json"))
            .unwrap();
        let count_before = backups_before.len();

        std::fs::write(d.path().join("system.json"), r#"{"version":"9.9"}"#).unwrap();
        mgr.reload_section(ConfigSection::System).unwrap();

        let backups_after = cm
            .backup_manager()
            .list_backups(d.path().join("system.json"))
            .unwrap();
        assert_eq!(
            backups_after.len(),
            count_before + 1,
            "successful reload should create exactly one backup of old value"
        );
    }

    /// First load (no old value) → success, no backup (nothing to back up).
    #[test]
    fn test_reload_section_first_load_no_backup() {
        let d = TempDir::new().unwrap();
        let cm = Arc::new(ConfigManager::new(d.path().to_path_buf()).unwrap());
        let cb = Arc::new(MockCallback::new());
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), cb);

        let backups_before = cm
            .backup_manager()
            .list_backups(d.path().join("system.json"))
            .unwrap();
        let count_before = backups_before.len();

        std::fs::write(d.path().join("system.json"), r#"{"version":"1.0"}"#).unwrap();
        mgr.reload_section(ConfigSection::System).unwrap();

        let backups_after = cm
            .backup_manager()
            .list_backups(d.path().join("system.json"))
            .unwrap();
        assert_eq!(
            backups_after.len(),
            count_before,
            "first load with no old value should not create any backup"
        );

        let val = cm.section(ConfigSection::System).unwrap();
        assert_eq!(val["version"], "1.0");
    }
}
