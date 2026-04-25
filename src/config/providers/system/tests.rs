//! Unit tests for SystemConfigData

use super::{
    AuthProfileEntryConfig, AuthProfilesConfig, BrowserConfig, CommandsConfig, CronConfig,
    HookEntryConfig, HooksConfig, HooksInternalConfig, MessagesConfig, MetaConfig, SessionConfig,
    SessionMaintenanceConfig, SystemConfigData, UpdateConfig, WizardConfig,
};
use crate::config::ConfigProvider;

fn minimal_config() -> SystemConfigData {
    SystemConfigData::default()
}

fn full_config() -> SystemConfigData {
    SystemConfigData {
        wizard: Some(WizardConfig {
            last_run_at: Some("2026-04-22T03:11:53.234Z".to_string()),
            last_run_version: Some("2026.4.15".to_string()),
            last_run_command: Some("configure".to_string()),
            last_run_mode: Some("local".to_string()),
        }),
        update: Some(UpdateConfig {
            check_on_start: false,
        }),
        meta: Some(MetaConfig {
            last_touched_version: Some("2026.4.15".to_string()),
            last_touched_at: Some("2026-04-22T03:11:53.549Z".to_string()),
        }),
        messages: Some(MessagesConfig {
            ack_reaction_scope: Some("group-mentions".to_string()),
        }),
        commands: Some(CommandsConfig {
            native: true,
            native_skills: true,
            restart: true,
            owner_display: Some("raw".to_string()),
        }),
        session: Some(SessionConfig {
            dm_scope: "per-account-channel-peer".to_string(),
            maintenance: SessionMaintenanceConfig {
                mode: "enforce".to_string(),
                prune_after: "7d".to_string(),
                max_entries: 500,
            },
        }),
        cron: Some(CronConfig { enabled: true }),
        hooks: Some(HooksConfig {
            internal: HooksInternalConfig {
                enabled: true,
                entries: std::collections::BTreeMap::from([
                    ("boot-md".to_string(), HookEntryConfig { enabled: true }),
                    (
                        "command-logger".to_string(),
                        HookEntryConfig { enabled: true },
                    ),
                    (
                        "session-memory".to_string(),
                        HookEntryConfig { enabled: true },
                    ),
                ]),
            },
        }),
        browser: Some(BrowserConfig {
            executable_path: Some("/usr/bin/google-chrome".to_string()),
            headless: true,
            default_profile: Some("openclaw".to_string()),
        }),
        auth: Some(AuthProfilesConfig {
            profiles: std::collections::BTreeMap::from([
                (
                    "minimax-portal:default".to_string(),
                    AuthProfileEntryConfig {
                        provider: "minimax-portal".to_string(),
                        mode: "oauth".to_string(),
                    },
                ),
                (
                    "deepseek:default".to_string(),
                    AuthProfileEntryConfig {
                        provider: "deepseek".to_string(),
                        mode: "api_key".to_string(),
                    },
                ),
                (
                    "zai:default".to_string(),
                    AuthProfileEntryConfig {
                        provider: "zai".to_string(),
                        mode: "api_key".to_string(),
                    },
                ),
            ]),
        }),
    }
}

#[test]
fn test_default_config_is_valid() {
    minimal_config()
        .validate()
        .expect("default config should be valid");
}

#[test]
fn test_default_config_is_default() {
    assert!(minimal_config().is_default());
}

#[test]
fn test_full_config_is_not_default() {
    assert!(!full_config().is_default());
}

#[test]
fn test_full_config_is_valid() {
    full_config()
        .validate()
        .expect("full config should be valid");
}

#[test]
fn test_invalid_maintenance_mode_fails() {
    let mut cfg = minimal_config();
    cfg.session = Some(SessionConfig {
        dm_scope: "per-account-channel-peer".to_string(),
        maintenance: SessionMaintenanceConfig {
            mode: "bad".to_string(),
            prune_after: "7d".to_string(),
            max_entries: 500,
        },
    });
    assert!(cfg.validate().is_err());
}

#[test]
fn test_invalid_dm_scope_fails() {
    let mut cfg = minimal_config();
    cfg.session = Some(SessionConfig {
        dm_scope: "bad".to_string(),
        ..Default::default()
    });
    assert!(cfg.validate().is_err());
}

#[test]
fn test_from_json_str_empty() {
    let cfg = SystemConfigData::from_json_str("{}").expect("empty JSON should parse");
    assert!(cfg.is_default());
}

#[test]
fn test_from_json_str_partial() {
    let json = r#"{"wizard":{"lastRunAt":"2026-04-22T03:11:53.234Z","lastRunVersion":"2026.4.15"},"update":{"checkOnStart":false},"commands":{"ownerDisplay":"raw"}}"#;
    let cfg = SystemConfigData::from_json_str(json).expect("partial JSON should parse");
    assert!(cfg.wizard.is_some());
    assert_eq!(
        cfg.wizard.as_ref().unwrap().last_run_version.as_deref(),
        Some("2026.4.15")
    );
    assert!(!cfg.update.as_ref().unwrap().check_on_start);
    assert_eq!(
        cfg.commands.as_ref().unwrap().owner_display.as_deref(),
        Some("raw")
    );
}

#[test]
fn test_from_json_str_with_hooks() {
    let json = r#"{"hooks":{"internal":{"enabled":true,"entries":{"boot-md":{"enabled":true}}}}}"#;
    let cfg = SystemConfigData::from_json_str(json).expect("hooks JSON should parse");
    let entries = &cfg.hooks.as_ref().unwrap().internal.entries;
    assert!(entries.contains_key("boot-md"));
    assert!(!entries.contains_key("nonexistent"));
}

#[test]
fn test_from_json_str_with_auth_profiles() {
    let json = r#"{"auth":{"profiles":{"minimax-portal:default":{"provider":"minimax-portal","mode":"oauth"}}}}"#;
    let cfg = SystemConfigData::from_json_str(json).expect("auth profiles JSON should parse");
    let entry = cfg
        .auth
        .as_ref()
        .unwrap()
        .profiles
        .get("minimax-portal:default")
        .unwrap();
    assert_eq!(entry.provider, "minimax-portal");
    assert_eq!(entry.mode, "oauth");
}

#[test]
fn test_from_json_str_invalid_json() {
    assert!(SystemConfigData::from_json_str("not json").is_err());
}

#[test]
fn test_config_path() {
    assert_eq!(
        SystemConfigData::config_path(),
        "openclaw.json (system section)"
    );
}

#[test]
fn test_version() {
    assert_eq!(minimal_config().version(), "1.0.0");
}

#[test]
fn test_serialize_camel_case() {
    let json = serde_json::to_string(&full_config()).expect("should serialize");
    assert!(json.contains('"'));
    assert!(
        !json.contains("last_run_at"),
        "should use camelCase not snake_case"
    );
    assert!(json.contains("lastRunAt"));
}

#[test]
fn test_serialize_roundtrip() {
    let original = full_config();
    let json = serde_json::to_string(&original).expect("should serialize");
    let parsed: SystemConfigData = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(parsed, original);
}

#[test]
fn test_from_file_valid() {
    let temp = tempfile::NamedTempFile::with_suffix(".json").unwrap();
    std::fs::write(
        temp.path(),
        r#"{"wizard":{"lastRunAt":"2026-04-22T03:11:53.234Z"}}"#,
    )
    .unwrap();
    let cfg = SystemConfigData::from_file(temp.path()).expect("should load from file");
    assert!(cfg.wizard.is_some());
    assert_eq!(
        cfg.wizard.as_ref().unwrap().last_run_at.as_deref(),
        Some("2026-04-22T03:11:53.234Z")
    );
}

#[test]
fn test_from_file_not_found() {
    assert!(SystemConfigData::from_file("/nonexistent/path/nowhere.json").is_err());
}

#[test]
fn test_wizard_config_default() {
    let w = WizardConfig::default();
    assert!(
        w.last_run_at.is_none()
            && w.last_run_version.is_none()
            && w.last_run_command.is_none()
            && w.last_run_mode.is_none()
    );
}

#[test]
fn test_update_config_default() {
    assert!(UpdateConfig::default().check_on_start);
}

#[test]
fn test_meta_config_default() {
    assert!(
        MetaConfig::default().last_touched_version.is_none()
            && MetaConfig::default().last_touched_at.is_none()
    );
}

#[test]
fn test_messages_config_default() {
    assert!(MessagesConfig::default().ack_reaction_scope.is_none());
}

#[test]
fn test_commands_config_default() {
    let c = CommandsConfig::default();
    assert!(c.native && c.native_skills && c.restart && c.owner_display.is_none());
}

#[test]
fn test_session_maintenance_config_default() {
    let m = SessionMaintenanceConfig::default();
    assert_eq!(m.mode, "enforce");
    assert_eq!(m.prune_after, "7d");
    assert_eq!(m.max_entries, 500);
}

#[test]
fn test_session_config_default() {
    let s = SessionConfig::default();
    assert_eq!(s.dm_scope, "per-account-channel-peer");
    assert_eq!(s.maintenance.mode, "enforce");
}

#[test]
fn test_cron_config_default() {
    assert!(CronConfig::default().enabled);
}

#[test]
fn test_hooks_internal_config_default() {
    let h = HooksInternalConfig::default();
    assert!(h.enabled && h.entries.is_empty());
}

#[test]
fn test_hooks_config_default() {
    let h = HooksConfig::default();
    assert!(h.internal.enabled && h.internal.entries.is_empty());
}

#[test]
fn test_browser_config_default() {
    let b = BrowserConfig::default();
    assert!(b.executable_path.is_none() && b.headless && b.default_profile.is_none());
}

#[test]
fn test_auth_profile_entry_config_default() {
    let a = AuthProfileEntryConfig::default();
    assert_eq!(a.provider, "");
    assert_eq!(a.mode, "");
}

#[test]
fn test_auth_profiles_config_default() {
    assert!(AuthProfilesConfig::default().profiles.is_empty());
}

#[test]
fn test_is_default_true_when_all_sub_structs_are_default() {
    let cfg = SystemConfigData {
        wizard: None,
        update: Some(UpdateConfig {
            check_on_start: true,
        }),
        meta: None,
        messages: None,
        commands: Some(CommandsConfig::default()),
        session: Some(SessionConfig::default()),
        cron: Some(CronConfig::default()),
        hooks: Some(HooksConfig::default()),
        browser: Some(BrowserConfig::default()),
        auth: None,
    };
    assert!(cfg.is_default());
}

#[test]
fn test_is_default_false_when_update_check_on_start_false() {
    let cfg = SystemConfigData {
        update: Some(UpdateConfig {
            check_on_start: false,
        }),
        ..Default::default()
    };
    assert!(!cfg.is_default());
}

#[test]
fn test_is_default_false_when_commands_native_false() {
    let cfg = SystemConfigData {
        commands: Some(CommandsConfig {
            native: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!cfg.is_default());
}

#[test]
fn test_is_default_false_when_commands_owner_display_set() {
    let cfg = SystemConfigData {
        commands: Some(CommandsConfig {
            owner_display: Some("raw".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!cfg.is_default());
}

#[test]
fn test_is_default_false_when_browser_headless_false() {
    let cfg = SystemConfigData {
        browser: Some(BrowserConfig {
            headless: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!cfg.is_default());
}

#[test]
fn test_is_default_false_when_hooks_internal_disabled() {
    let cfg = SystemConfigData {
        hooks: Some(HooksConfig {
            internal: HooksInternalConfig {
                enabled: false,
                ..Default::default()
            },
        }),
        ..Default::default()
    };
    assert!(!cfg.is_default());
}

#[test]
fn test_is_default_false_when_cron_disabled() {
    let cfg = SystemConfigData {
        cron: Some(CronConfig { enabled: false }),
        ..Default::default()
    };
    assert!(!cfg.is_default());
}

#[test]
fn test_validate_maintenance_mode_warn_valid() {
    let cfg = SystemConfigData {
        session: Some(SessionConfig {
            dm_scope: "per-account-channel-peer".to_string(),
            maintenance: SessionMaintenanceConfig {
                mode: "warn".to_string(),
                prune_after: "7d".to_string(),
                max_entries: 500,
            },
        }),
        ..Default::default()
    };
    cfg.validate().expect("mode=warn should be valid");
}

#[test]
fn test_validate_maintenance_mode_off_valid() {
    let cfg = SystemConfigData {
        session: Some(SessionConfig {
            dm_scope: "per-account-channel-peer".to_string(),
            maintenance: SessionMaintenanceConfig {
                mode: "off".to_string(),
                prune_after: "7d".to_string(),
                max_entries: 500,
            },
        }),
        ..Default::default()
    };
    cfg.validate().expect("mode=off should be valid");
}

#[test]
fn test_validate_dm_scope_per_channel_peer_valid() {
    let cfg = SystemConfigData {
        session: Some(SessionConfig {
            dm_scope: "per-channel-peer".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    cfg.validate()
        .expect("dmScope=per-channel-peer should be valid");
}

#[test]
fn test_validate_dm_scope_per_peer_valid() {
    let cfg = SystemConfigData {
        session: Some(SessionConfig {
            dm_scope: "per-peer".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    cfg.validate().expect("dmScope=per-peer should be valid");
}

#[test]
fn test_validate_dm_scope_main_valid() {
    let cfg = SystemConfigData {
        session: Some(SessionConfig {
            dm_scope: "main".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    cfg.validate().expect("dmScope=main should be valid");
}

#[test]
fn test_validate_empty_session_valid() {
    SystemConfigData::default()
        .validate()
        .expect("no session config should be valid");
}
