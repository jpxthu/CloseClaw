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
}
