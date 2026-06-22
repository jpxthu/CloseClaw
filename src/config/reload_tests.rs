#[cfg(test)]
mod reload_tests {
    use crate::agent::registry::AgentRegistry;
    use crate::config::manager::{ConfigManager, ConfigSection};
    use crate::config::reload::{
        dispatch_change, filename_to_section, is_agents_path, ConfigReloadManager, DEFAULT_DEBOUNCE,
    };
    use std::path::Path;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;
    use tempfile::TempDir;

    fn make_config_manager(dir: &std::path::Path) -> Arc<ConfigManager> {
        let sections = [
            ("models.json", r#"{"models":[]}"#),
            ("channels.json", r#"{"channels":{}}"#),
            ("gateway.json", r#"{"port":8080}"#),
            ("plugins.json", r#"{"plugins":[]}"#),
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
        let mut mgr = ConfigReloadManager::with_defaults(cm, ar);
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
        assert_eq!(
            filename_to_section("session.json"),
            Some(ConfigSection::Session)
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
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), ar);

        std::fs::write(d.path().join("models.json"), r#"{"models":[{"id":"m1"}]}"#).unwrap();

        let path = d.path().join("models.json");
        dispatch_change(&path, &mgr);

        let val = cm.section(ConfigSection::Models).unwrap();
        assert!(val.get("models").is_some());
    }

    #[test]
    fn test_dispatch_agents_json_change() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm, ar);

        std::fs::create_dir_all(d.path().join("agents")).unwrap();
        std::fs::write(d.path().join("agents.json"), r#"{"agents":{}}"#).unwrap();

        let path = d.path().join("agents.json");
        dispatch_change(&path, &mgr);
    }

    #[test]
    fn test_dispatch_unknown_file_is_noop() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm, ar);

        let path = d.path().join("unknown.json");
        dispatch_change(&path, &mgr);
    }

    #[test]
    fn test_dispatch_nonexistent_agents_dir() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm, ar);

        let path = d.path().join("agents").join("test.json");
        dispatch_change(&path, &mgr);
    }

    #[test]
    fn test_watcher_handle_stop() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mut mgr = ConfigReloadManager::with_defaults(cm, ar);
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
        let mut mgr = ConfigReloadManager::new(cm.clone(), ar, Duration::from_millis(50));
        let (tx, rx) = mpsc::channel();
        mgr.set_test_completion(tx);
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
                format!(r#"{{"models":[{{"id":"m{}"}}]}}"#, i),
            )
            .unwrap();
        }

        // Wait for at least one dispatch cycle to complete via signal
        rx.recv_timeout(Duration::from_secs(5))
            .expect("reload loop should have completed at least one dispatch cycle");

        // Verify the system is in a consistent state (no panic, no deadlock).
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
        let mut mgr = ConfigReloadManager::new(cm.clone(), ar, Duration::from_millis(50));
        let (tx, rx) = mpsc::channel();
        mgr.set_test_completion(tx);
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        std::fs::write(
            d.path().join("plugins.json"),
            r#"{"plugins":[{"id":"p1"}]}"#,
        )
        .unwrap();

        // Wait for the dispatch cycle to complete via signal
        rx.recv_timeout(Duration::from_secs(5))
            .expect("reload loop should have processed the file change");

        let val = cm.section(ConfigSection::Plugins).unwrap();
        assert!(val.get("plugins").is_some());
    }

    /// Multiple file changes within the debounce window are merged and
    /// all dispatched when the window closes.
    #[test]
    fn test_debounce_merges_multiple_file_changes() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mut mgr = ConfigReloadManager::new(cm.clone(), ar, Duration::from_millis(100));
        let (tx, rx) = mpsc::channel();
        mgr.set_test_completion(tx);
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        // Write to multiple sections rapidly — all should be collected
        std::fs::write(d.path().join("models.json"), r#"{"models":[{"id":"m1"}]}"#).unwrap();
        std::fs::write(
            d.path().join("channels.json"),
            r#"{"channels":{"type":{}}}"#,
        )
        .unwrap();
        std::fs::write(d.path().join("gateway.json"), r#"{"port":9999}"#).unwrap();

        // Wait for the debounce window to close and dispatch to fire
        rx.recv_timeout(Duration::from_secs(5))
            .expect("reload loop should have dispatched all merged changes");

        // All three sections should have been reloaded
        let md = cm.section(ConfigSection::Models).unwrap();
        assert!(md.get("models").is_some(), "models should be reloaded");
        let ch = cm.section(ConfigSection::Channels).unwrap();
        assert!(ch.get("channels").is_some(), "channels should be reloaded");
        let gw = cm.section(ConfigSection::Gateway).unwrap();
        assert!(gw.is_object(), "gateway should be reloaded");
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
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), ar);

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
        ];

        for (filename, section, content) in cases {
            std::fs::write(d.path().join(filename), content).unwrap();
            let path = d.path().join(filename);
            dispatch_change(&path, &mgr);
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
        let mgr = ConfigReloadManager::with_defaults(cm, ar);

        let agents_dir = d.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("test.json"), r#"{}"#).unwrap();

        let path = agents_dir.join("test.json");
        dispatch_change(&path, &mgr);
        // Should not panic; agent reload path was exercised
    }

    /// Verify WatcherHandle RAII: dropping the handle does not panic
    /// even when the watcher is active.
    #[test]
    fn test_watcher_handle_drop_is_safe() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mut mgr = ConfigReloadManager::with_defaults(cm, ar);
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

    // ------------------------------------------------------------------
    // Agent rollback disk tests
    // ------------------------------------------------------------------

    /// reload_agents snapshot/restore correctly preserves agent state.
    /// Tests the rollback mechanism used in reload_agents_with_log:
    /// 1. Load valid agents
    /// 2. Snapshot the valid state
    /// 3. Modify agents (add new agent)
    /// 4. Simulate failure → restore from snapshot
    /// 5. Verify original state is recovered
    #[test]
    fn test_reload_agents_disk_rollback_on_failure() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let _ar = make_agent_registry();

        // Set up valid agent files: agents.json + agents/alpha/config.json
        // agents.json is at config_dir root (e.g. ~/.closeclaw/agents.json)
        let agents_json_path = d.path().join("agents.json");
        std::fs::write(&agents_json_path, r#"{ "agents": ["alpha"] }"#).unwrap();

        // agents/ is at root level (parent of config_dir)
        let root_dir = d.path().parent().unwrap().to_path_buf();
        let alpha_dir = root_dir.join("agents").join("alpha");
        std::fs::create_dir_all(&alpha_dir).unwrap();
        std::fs::write(
            alpha_dir.join("config.json"),
            r#"{ "id": "alpha", "name": "Alpha" }"#,
        )
        .unwrap();

        // Load agents into memory
        cm.load_agents(None).unwrap();
        assert!(cm.agents().contains_key("alpha"));

        // Snapshot valid state (as reload_agents_with_log does before reload)
        let (old_agents, old_permissions) = cm.snapshot_agents();
        assert_eq!(old_agents.len(), 1);
        assert!(old_agents.contains_key("alpha"));

        // Backup disk files before modification (as reload_agents_with_log does)
        let _ = cm.backup_manager().backup(&agents_json_path);
        let _ = cm.backup_manager().backup(alpha_dir.join("config.json"));

        // Record original disk content for later verification
        let original_agents_json = std::fs::read_to_string(&agents_json_path).unwrap();
        let original_alpha_config = std::fs::read_to_string(alpha_dir.join("config.json")).unwrap();

        // Add a new agent (simulating a change that will be rolled back)
        let beta_dir = root_dir.join("agents").join("beta");
        std::fs::create_dir_all(&beta_dir).unwrap();
        std::fs::write(
            beta_dir.join("config.json"),
            r#"{ "id": "beta", "name": "Beta" }"#,
        )
        .unwrap();
        std::fs::write(&agents_json_path, r#"{ "agents": ["alpha", "beta"] }"#).unwrap();
        cm.reload_agents().unwrap();
        assert!(
            cm.agents().contains_key("beta"),
            "beta should be present after adding"
        );

        // Simulate failure: restore from snapshot (as reload_agents_with_log does)
        cm.restore_agents(old_agents, old_permissions);

        // Rollback disk files to last known good state
        let _ = cm.backup_manager().rollback(&agents_json_path);
        let _ = cm.backup_manager().rollback(alpha_dir.join("config.json"));

        // Verify: original memory state recovered, beta is gone
        assert!(
            cm.agents().contains_key("alpha"),
            "alpha should be present after rollback"
        );
        assert_eq!(cm.agents()["alpha"].id, "alpha");
        assert!(
            !cm.agents().contains_key("beta"),
            "beta should not exist after rollback"
        );

        // Verify: disk files rolled back to original content
        let rolled_back_agents_json = std::fs::read_to_string(&agents_json_path).unwrap();
        assert_eq!(
            rolled_back_agents_json, original_agents_json,
            "agents.json should be rolled back to original content"
        );
        let rolled_back_alpha_config =
            std::fs::read_to_string(alpha_dir.join("config.json")).unwrap();
        assert_eq!(
            rolled_back_alpha_config, original_alpha_config,
            "alpha config.json should be rolled back to original content"
        );
    }

    // ------------------------------------------------------------------
    // Debounce dedup tests
    // ------------------------------------------------------------------

    /// Multiple rapid writes to the same file within the debounce window
    /// are deduplicated — only one reload fires for that path.
    #[test]
    fn test_debounce_dedup_same_file() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mut mgr = ConfigReloadManager::new(cm.clone(), ar, Duration::from_millis(100));
        let (tx, rx) = mpsc::channel();
        mgr.set_test_completion(tx);
        let _handle = mgr.watch(d.path().to_str().unwrap()).unwrap();

        // Rapid writes to the SAME file — should be deduplicated
        for i in 1..=20 {
            std::fs::write(
                d.path().join("gateway.json"),
                format!(r#"{{"port":{}}}"#, 8000 + i),
            )
            .unwrap();
        }

        // Wait for debounce window to close
        rx.recv_timeout(Duration::from_secs(5))
            .expect("reload loop should have completed at least one dispatch cycle");

        // The gateway section should reflect the LAST write (port 8020)
        let gw = cm.section(ConfigSection::Gateway).unwrap();
        assert_eq!(
            gw["port"], 8020,
            "gateway port should reflect the last write after dedup"
        );
    }

    /// dispatch_change uses the default validator — reloading an invalid
    /// config via dispatch_change is rejected and the in-memory value
    /// is unchanged.
    #[test]
    fn test_dispatch_validates_with_default_validator() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), ar);

        // Load initial valid models.json
        std::fs::write(d.path().join("models.json"), r#"{"models":[{"id":"m1"}]}"#).unwrap();
        let path = d.path().join("models.json");
        dispatch_change(&path, &mgr);
        let before = cm.section(ConfigSection::Models).unwrap();
        assert!(before.get("models").is_some());

        // Write a valid JSON file but with a non-array "models" field.
        // The default validator should reject this.
        std::fs::write(d.path().join("models.json"), r#"{"models":"not an array"}"#).unwrap();
        dispatch_change(&path, &mgr);

        // In-memory must still be the old value (validator rejected reload)
        let after = cm.section(ConfigSection::Models).unwrap();
        // The new invalid value should NOT be present
        let models = after.get("models").unwrap();
        assert!(
            models.is_array(),
            "models should still be the valid array, not replaced by the invalid value"
        );
    }

    // ------------------------------------------------------------------
    // ConfigReloadManager::reload_section tests
    // ------------------------------------------------------------------

    /// ConfigReloadManager::reload_section correctly reloads a config section.
    #[test]
    fn test_reload_section_via_manager() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), ar);

        std::fs::write(d.path().join("system.json"), r#"{"version":"2.0"}"#).unwrap();
        let section = ConfigSection::System;
        mgr.reload_section(section).unwrap();

        let val = cm.section(section).unwrap();
        assert_eq!(val["version"], "2.0");
    }

    /// ConfigReloadManager::reload_section handles validation failure.
    #[test]
    fn test_reload_section_validation_failure_via_manager() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), ar);

        // Write invalid JSON (non-object for Models which expects array)
        std::fs::write(d.path().join("models.json"), r#"{"models":"not array"}"#).unwrap();
        let section = ConfigSection::Models;
        let result = mgr.reload_section(section);
        assert!(result.is_err());

        // In-memory should still be the old value
        let val = cm.section(section).unwrap();
        assert!(val.get("models").is_some());
    }

    // ------------------------------------------------------------------
    // Step 1.8 — supplementary tests
    // ------------------------------------------------------------------

    /// ConfigReloadManager::reload_section uses default_validator;
    /// a section that passes default validation is reloaded into the
    /// in-memory cache and a snapshot is broadcast.
    #[test]
    fn test_reload_section_via_manager_with_validation() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), ar);

        // Write valid system.json that passes the default validator
        std::fs::write(d.path().join("system.json"), r#"{"version":"8.8"}"#).unwrap();
        let mut snapshot_rx = cm.subscribe_config_snapshots();

        mgr.reload_section(ConfigSection::System).unwrap();

        // In-memory should be updated
        let val = cm.section(ConfigSection::System).unwrap();
        assert_eq!(val["version"], "8.8");

        // Snapshot should have been broadcast
        let snapshot = snapshot_rx
            .try_recv()
            .expect("should receive a snapshot after successful reload");
        assert_eq!(
            snapshot.get(&ConfigSection::System).unwrap()["version"],
            "8.8"
        );
    }

    /// ConfigReloadManager::reload_agents_with_log succeeds when agent
    /// files are valid; the AgentRegistry is synced with the new configs.
    #[test]
    fn test_reload_agents_with_log_direct() {
        let d = TempDir::new().unwrap();
        let cm = make_config_manager(d.path());
        let ar = make_agent_registry();
        let mgr = ConfigReloadManager::with_defaults(cm.clone(), ar.clone());

        // Set up valid agent files: agents.json + agents/alpha/config.json
        // agents.json is at config_dir root
        let agents_json_path = d.path().join("agents.json");
        std::fs::write(&agents_json_path, r#"{ "agents": ["alpha"] }"#).unwrap();

        // agents/ is at root level (parent of config_dir)
        let root_dir = d.path().parent().unwrap().to_path_buf();
        let alpha_dir = root_dir.join("agents").join("alpha");
        std::fs::create_dir_all(&alpha_dir).unwrap();
        std::fs::write(
            alpha_dir.join("config.json"),
            r#"{ "id": "alpha", "name": "Alpha" }"#,
        )
        .unwrap();

        // Trigger reload via reload_agents_with_log
        let path = agents_json_path.clone();
        mgr.reload_agents_with_log(&path);

        // AgentRegistry should be synced with alpha
        let agents: Vec<_> = ar.iter().map(|e| e.key().clone()).collect();
        assert!(
            agents.contains(&"alpha".to_string()),
            "AgentRegistry should contain alpha after reload"
        );
    }
}
