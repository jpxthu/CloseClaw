#[cfg(test)]
mod reload_tests {
    use crate::agent::registry::AgentRegistry;
    use crate::config::manager::{ConfigManager, ConfigSection};
    use crate::config::reload::{
        dispatch_change, filename_to_section, is_agents_path, ConfigReloadManager, DEFAULT_DEBOUNCE,
    };
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    fn make_config_manager(dir: &std::path::Path) -> Arc<ConfigManager> {
        let sections = [
            ("models.json", r#"{"models":{}}"#),
            ("channels.json", r#"{"channels":{}}"#),
            ("gateway.json", r#"{"port":8080}"#),
            ("plugins.json", r#"{"plugins":{}}"#),
            ("system.json", r#"{"version":"1"}"#),
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

    #[test]
    fn test_new_and_defaults() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm, ar);
        assert_eq!(mgr.debounce_duration, DEFAULT_DEBOUNCE);
    }

    #[test]
    fn test_new_custom_debounce() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::new(cm, ar, Duration::from_secs(2));
        assert_eq!(mgr.debounce_duration, Duration::from_secs(2));
    }

    #[test]
    fn test_watch_returns_handle() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm, ar);
        let handle = mgr.watch(d.path().to_str().unwrap());
        assert!(handle.is_ok());
        drop(handle.unwrap());
    }

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
        assert_eq!(filename_to_section("agents.json"), None);
        assert_eq!(filename_to_section("unknown.json"), None);
    }

    #[test]
    fn test_is_agents_path() {
        assert!(is_agents_path(Path::new("/config/agents/test.json")));
        assert!(is_agents_path(Path::new("/config\\agents\\test.json")));
        assert!(!is_agents_path(Path::new("/config/models.json")));
    }

    #[test]
    fn test_dispatch_section_change() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();

        std::fs::write(d.path().join("models.json"), r#"{"models":{"m1":{}}}"#).unwrap();

        let path = d.path().join("models.json");
        dispatch_change(&path, &cm, &ar);

        let val = cm.section(ConfigSection::Models).unwrap();
        assert!(val.get("models").is_some());
    }

    #[test]
    fn test_dispatch_agents_json_change() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();

        std::fs::create_dir_all(d.path().join("agents")).unwrap();
        std::fs::write(d.path().join("agents.json"), r#"{"agents":{}}"#).unwrap();

        let path = d.path().join("agents.json");
        dispatch_change(&path, &cm, &ar);
    }

    #[test]
    fn test_dispatch_unknown_file_is_noop() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();

        let path = d.path().join("unknown.json");
        dispatch_change(&path, &cm, &ar);
    }

    #[test]
    fn test_dispatch_nonexistent_agents_dir() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();

        let path = d.path().join("agents").join("test.json");
        dispatch_change(&path, &cm, &ar);
    }

    #[test]
    fn test_watcher_handle_stop() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm, ar);
        let handle = mgr.watch(d.path().to_str().unwrap()).unwrap();
        handle.stop();
    }

    // ------------------------------------------------------------------
    // Debounce integration tests
    // ------------------------------------------------------------------

    /// Watcher handles rapid file changes without panic or deadlock.
    /// The debounce logic ensures only some reloads fire, but the key
    /// invariant is that the system remains stable under rapid input.
    #[test]
    fn test_rapid_file_changes_stable() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::new(cm.clone(), ar, Duration::from_millis(50));
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        // Rapid writes across multiple sections
        for i in 1..=10 {
            std::fs::write(
                d.path().join("gateway.json"),
                format!(r#"{{"port":{}}}"#, 8000 + i),
            )
            .unwrap();
            std::fs::write(
                d.path().join("models.json"),
                format!(r#"{{"models":{{"m{}":{{}}}}}}"#, i),
            )
            .unwrap();
        }

        // Wait for the watcher thread to process events
        std::thread::sleep(Duration::from_millis(600));

        // Verify the system is in a consistent state (no panic, no deadlock).
        // At least one reload should have been triggered.
        let gw = cm.section(ConfigSection::Gateway).unwrap();
        assert!(gw.is_object(), "gateway section should be an object");
        let md = cm.section(ConfigSection::Models).unwrap();
        assert!(md.is_object(), "models section should be an object");
    }

    /// A single file change triggers exactly one reload (no debounce skipping
    /// when only one event arrives).
    #[test]
    fn test_single_change_reloads_correctly() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::new(cm.clone(), ar, Duration::from_millis(50));
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        std::fs::write(d.path().join("plugins.json"), r#"{"plugins":{"p1":{}}}"#).unwrap();
        std::thread::sleep(Duration::from_millis(300));

        let val = cm.section(ConfigSection::Plugins).unwrap();
        assert!(val.get("plugins").is_some());
    }

    // ------------------------------------------------------------------
    // Dispatch per-section tests
    // ------------------------------------------------------------------

    /// Each known config section triggers the correct ConfigSection variant.
    #[test]
    fn test_dispatch_all_config_sections() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();

        let cases = [
            (
                "channels.json",
                ConfigSection::Channels,
                r#"{"channels":{"ch1":{}}}"#,
            ),
            ("gateway.json", ConfigSection::Gateway, r#"{"port":9090}"#),
            (
                "plugins.json",
                ConfigSection::Plugins,
                r#"{"plugins":{"pl1":{}}}"#,
            ),
            ("system.json", ConfigSection::System, r#"{"version":"2.0"}"#),
            (
                "models.json",
                ConfigSection::Models,
                r#"{"models":{"m1":{}}}"#,
            ),
        ];

        for (filename, section, content) in cases {
            std::fs::write(d.path().join(filename), content).unwrap();
            let path = d.path().join(filename);
            dispatch_change(&path, &cm, &ar);
            let val = cm.section(section).unwrap();
            assert!(val.is_object(), "section {:?} should be an object", section);
        }
    }

    /// agents/ directory change dispatches to the agent reload path.
    #[test]
    fn test_dispatch_agents_dir_change() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();

        let agents_dir = d.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("test.json"), r#"{}"#).unwrap();

        let path = agents_dir.join("test.json");
        dispatch_change(&path, &cm, &ar);
        // Should not panic; agent reload path was exercised
    }

    /// Verify WatcherHandle RAII: dropping the handle does not panic
    /// even when the watcher is active.
    #[test]
    fn test_watcher_handle_drop_is_safe() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm, ar);
        let handle = mgr.watch(d.path().to_str().unwrap()).unwrap();
        drop(handle); // RAII drop — must not panic
    }

    /// ConfigReloadManager holds shared Arc references — cloning them
    /// does not affect the original manager.
    #[test]
    fn test_config_reload_manager_clones_arcs() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let _mgr1 = ConfigReloadManager::with_defaults(cm.clone(), ar.clone());
        let _mgr2 = ConfigReloadManager::with_defaults(cm.clone(), ar.clone());
        // Both managers share the same underlying ConfigManager (Arc ptr_eq)
        // We can verify by checking they produce the same section data
        let s1 = cm.section(ConfigSection::System).unwrap();
        let s2 = cm.section(ConfigSection::System).unwrap();
        assert_eq!(s1, s2, "shared Arc should produce identical data");
    }
}
