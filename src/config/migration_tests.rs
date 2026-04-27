#[cfg(test)]
mod tests {
    use crate::config::migration::*;
    use std::fs;
    use tempfile::TempDir;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn temp_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    fn openclaw_json(tmp: &TempDir) -> PathBuf {
        tmp.path().join("openclaw.json")
    }

    fn config_dir(tmp: &TempDir) -> PathBuf {
        tmp.path().join("config")
    }

    // -------------------------------------------------------------------------
    // migrate_if_needed: conditions
    // -------------------------------------------------------------------------

    #[test]
    fn test_no_migration_when_openclaw_missing() {
        let tmp = temp_dir();
        // openclaw.json does not exist
        let result = migrate_if_needed(openclaw_json(&tmp), config_dir(&tmp)).unwrap();
        assert!(!result);
        assert!(!config_dir(&tmp).exists());
    }

    #[test]
    fn test_no_migration_when_config_exists() {
        let tmp = temp_dir();
        // Create openclaw.json
        fs::write(openclaw_json(&tmp), "{}").unwrap();
        // Create config/ already
        fs::create_dir(config_dir(&tmp)).unwrap();
        let result = migrate_if_needed(openclaw_json(&tmp), config_dir(&tmp)).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_no_migration_when_marker_exists() {
        let tmp = temp_dir();
        fs::write(openclaw_json(&tmp), "{}").unwrap();
        let cfg = config_dir(&tmp);
        fs::create_dir(&cfg).unwrap();
        fs::write(cfg.join("openclaw_migrated"), "").unwrap();
        let result = migrate_if_needed(openclaw_json(&tmp), config_dir(&tmp)).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_no_migration_when_openclaw_json_missing() {
        let tmp = temp_dir();
        let result = migrate_if_needed("/nonexistent/openclaw.json", config_dir(&tmp)).unwrap();
        assert!(!result);
    }

    // -------------------------------------------------------------------------
    // migrate_if_needed: malformed JSON
    // -------------------------------------------------------------------------

    #[test]
    fn test_migration_aborts_on_malformed_json() {
        let tmp = temp_dir();
        fs::write(openclaw_json(&tmp), "not valid json {{{").unwrap();
        let result = migrate_if_needed(openclaw_json(&tmp), config_dir(&tmp));
        assert!(matches!(result, Err(ConfigMigrationError::MalformedJson)));
        // config/ must NOT be created
        assert!(!config_dir(&tmp).exists());
    }

    // -------------------------------------------------------------------------
    // migrate_if_needed: full migration
    // -------------------------------------------------------------------------

    #[test]
    fn test_full_migration() {
        let tmp = temp_dir();
        let openclaw = r#"{
            "models": {"mode":"merge","providers":{"openai":{"apiKey":"sk-123"}}},
            "channels": {"feishu":{"enabled":true}},
            "bindings": [{"agentId":"a1","match":{"channel":"feishu"}}],
            "gateway": {"port":3000},
            "plugins": {"enabled":true},
            "wizard": {"lastRunAt":"2024-01-01"},
            "update": {"checkOnStart":false},
            "auth": {
                "profiles": {
                    "openai": {"apiKey":"sk-secret"},
                    "feishu": {"appId":"cli_abc","appSecret":"sec123"}
                }
            }
        }"#;
        fs::write(openclaw_json(&tmp), openclaw).unwrap();

        let result = migrate_if_needed(openclaw_json(&tmp), config_dir(&tmp)).unwrap();
        assert!(result);

        let cfg = config_dir(&tmp);

        // models.json
        let models: Value =
            serde_json::from_slice(&fs::read(cfg.join("models.json")).unwrap()).unwrap();
        assert_eq!(models["mode"], "merge");

        // channels.json
        let channels: Value =
            serde_json::from_slice(&fs::read(cfg.join("channels.json")).unwrap()).unwrap();
        assert_eq!(channels["channels"]["feishu"]["enabled"], true);
        assert_eq!(channels["bindings"][0]["agentId"], "a1");

        // gateway.json
        let gateway: Value =
            serde_json::from_slice(&fs::read(cfg.join("gateway.json")).unwrap()).unwrap();
        assert_eq!(gateway["port"], 3000);

        // plugins.json
        let plugins: Value =
            serde_json::from_slice(&fs::read(cfg.join("plugins.json")).unwrap()).unwrap();
        assert_eq!(plugins["enabled"], true);

        // system.json
        let system: Value =
            serde_json::from_slice(&fs::read(cfg.join("system.json")).unwrap()).unwrap();
        assert_eq!(system["wizard"]["lastRunAt"], "2024-01-01");
        assert_eq!(system["update"]["checkOnStart"], false);
        // auth.profiles present, no apiKey at top level of auth
        assert!(system["auth"]["profiles"].is_object());
        assert!(system["auth"].get("apiKey").is_none());

        // credentials/
        let openai_creds: Value =
            serde_json::from_slice(&fs::read(cfg.join("credentials/openai.json")).unwrap())
                .unwrap();
        assert_eq!(openai_creds["provider"], "openai");
        assert_eq!(openai_creds["apiKey"], "sk-secret");

        let feishu_creds: Value =
            serde_json::from_slice(&fs::read(cfg.join("credentials/feishu.json")).unwrap())
                .unwrap();
        assert_eq!(feishu_creds["provider"], "feishu");
        assert_eq!(feishu_creds["appId"], "cli_abc");
        assert_eq!(feishu_creds["appSecret"], "sec123");

        // openclaw.json.bak exists
        assert!(openclaw_json(&tmp).with_extension("json.bak").exists());
        // openclaw.json is gone
        assert!(!openclaw_json(&tmp).exists());
        // marker exists
        assert!(cfg.join("openclaw_migrated").exists());
        // .backups dir exists
        assert!(cfg.join(".backups").is_dir());
    }

    // -------------------------------------------------------------------------
    // migrate_if_needed: partial fields missing
    // -------------------------------------------------------------------------

    #[test]
    fn test_migration_with_missing_fields() {
        let tmp = temp_dir();
        // Only some fields present
        let openclaw = r#"{
            "gateway": {"port":4000}
        }"#;
        fs::write(openclaw_json(&tmp), openclaw).unwrap();

        let result = migrate_if_needed(openclaw_json(&tmp), config_dir(&tmp)).unwrap();
        assert!(result);

        let cfg = config_dir(&tmp);

        // Missing fields → empty objects
        let models: Value =
            serde_json::from_slice(&fs::read(cfg.join("models.json")).unwrap()).unwrap();
        assert_eq!(models, serde_json::json!({}));

        let channels: Value =
            serde_json::from_slice(&fs::read(cfg.join("channels.json")).unwrap()).unwrap();
        assert_eq!(channels["channels"], serde_json::json!({}));
        assert_eq!(channels["bindings"], serde_json::json!([]));

        let gateway: Value =
            serde_json::from_slice(&fs::read(cfg.join("gateway.json")).unwrap()).unwrap();
        assert_eq!(gateway["port"], 4000);

        let plugins: Value =
            serde_json::from_slice(&fs::read(cfg.join("plugins.json")).unwrap()).unwrap();
        assert_eq!(plugins, serde_json::json!({}));

        let system: Value =
            serde_json::from_slice(&fs::read(cfg.join("system.json")).unwrap()).unwrap();
        assert_eq!(system, serde_json::json!({}));

        // No credentials dir
        assert!(!cfg.join("credentials").exists());
    }

    // -------------------------------------------------------------------------
    // migrate_if_needed: no auth.profiles
    // -------------------------------------------------------------------------

    #[test]
    fn test_migration_no_auth_profiles() {
        let tmp = temp_dir();
        let openclaw = r#"{
            "gateway": {"port":5000},
            "auth": {"someOtherField": "value"}
        }"#;
        fs::write(openclaw_json(&tmp), openclaw).unwrap();

        let result = migrate_if_needed(openclaw_json(&tmp), config_dir(&tmp)).unwrap();
        assert!(result);

        let cfg = config_dir(&tmp);
        // No credentials directory should be created
        assert!(!cfg.join("credentials").exists());
    }

    // -------------------------------------------------------------------------
    // migrate_if_needed: multiple credential providers
    // -------------------------------------------------------------------------

    #[test]
    fn test_migration_multiple_credential_providers() {
        let tmp = temp_dir();
        let openclaw = r#"{
            "auth": {
                "profiles": {
                    "provider_a": {"apiKey": "key-a"},
                    "provider_b": {"appId": "id-b", "appSecret": "secret-b"},
                    "provider_c": {"apiKey": "key-c", "extra": "ignored"}
                }
            }
        }"#;
        fs::write(openclaw_json(&tmp), openclaw).unwrap();

        let result = migrate_if_needed(openclaw_json(&tmp), config_dir(&tmp)).unwrap();
        assert!(result);

        let cfg = config_dir(&tmp);
        let creds_dir = cfg.join("credentials");
        assert!(creds_dir.is_dir());

        let a: Value =
            serde_json::from_slice(&fs::read(creds_dir.join("provider_a.json")).unwrap()).unwrap();
        assert_eq!(a["provider"], "provider_a");
        assert_eq!(a["apiKey"], "key-a");

        let b: Value =
            serde_json::from_slice(&fs::read(creds_dir.join("provider_b.json")).unwrap()).unwrap();
        assert_eq!(b["provider"], "provider_b");
        assert_eq!(b["appId"], "id-b");
        assert_eq!(b["appSecret"], "secret-b");

        // extra field should NOT be in credentials
        let c: Value =
            serde_json::from_slice(&fs::read(creds_dir.join("provider_c.json")).unwrap()).unwrap();
        assert_eq!(c["apiKey"], "key-c");
        assert!(c.get("extra").is_none());
    }
}
