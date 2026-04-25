//! System JSON ConfigProvider
//!
//! Loads and validates the system section of openclaw.json.
//! Covers: wizard, update, meta, messages, commands, session, cron,
//!         hooks, browser, auth (profiles only — no apiKey).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::providers::ConfigError;
use crate::config::ConfigProvider;

// ---------------------------------------------------------------------------
// Sub-config structs
// ---------------------------------------------------------------------------

/// Wizard run state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WizardConfig {
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub last_run_version: Option<String>,
    #[serde(default)]
    pub last_run_command: Option<String>,
    #[serde(default)]
    pub last_run_mode: Option<String>,
}

impl Default for WizardConfig {
    fn default() -> Self {
        Self {
            last_run_at: None,
            last_run_version: None,
            last_run_command: None,
            last_run_mode: None,
        }
    }
}

/// Update check settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateConfig {
    #[serde(default)]
    pub check_on_start: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check_on_start: true,
        }
    }
}

/// Meta / version touch tracking.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MetaConfig {
    #[serde(default)]
    pub last_touched_version: Option<String>,
    #[serde(default)]
    pub last_touched_at: Option<String>,
}

impl Default for MetaConfig {
    fn default() -> Self {
        Self {
            last_touched_version: None,
            last_touched_at: None,
        }
    }
}

/// Message acknowledgement settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MessagesConfig {
    #[serde(default)]
    pub ack_reaction_scope: Option<String>,
}

impl Default for MessagesConfig {
    fn default() -> Self {
        Self {
            ack_reaction_scope: None,
        }
    }
}

/// Built-in command enable/disable flags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandsConfig {
    #[serde(default)]
    pub native: bool,
    #[serde(default)]
    pub native_skills: bool,
    #[serde(default)]
    pub restart: bool,
    #[serde(default)]
    pub owner_display: Option<String>,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            native: true,
            native_skills: true,
            restart: true,
            owner_display: None,
        }
    }
}

/// Session maintenance settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMaintenanceConfig {
    #[serde(default = "def_maint_mode")]
    pub mode: String,
    #[serde(default = "def_maint_prune")]
    pub prune_after: String,
    #[serde(default = "def_maint_entries")]
    pub max_entries: u32,
}

fn def_maint_mode() -> String {
    "enforce".to_string()
}
fn def_maint_prune() -> String {
    "7d".to_string()
}
fn def_maint_entries() -> u32 {
    500
}

impl Default for SessionMaintenanceConfig {
    fn default() -> Self {
        Self {
            mode: def_maint_mode(),
            prune_after: def_maint_prune(),
            max_entries: def_maint_entries(),
        }
    }
}

/// Session configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    #[serde(default = "def_dm_scope")]
    pub dm_scope: String,
    #[serde(default)]
    pub maintenance: SessionMaintenanceConfig,
}

fn def_dm_scope() -> String {
    "per-account-channel-peer".to_string()
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            dm_scope: def_dm_scope(),
            maintenance: SessionMaintenanceConfig::default(),
        }
    }
}

/// Cron / scheduled task toggle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CronConfig {
    #[serde(default)]
    pub enabled: bool,
}

impl Default for CronConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Individual hook entry record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookEntryConfig {
    #[serde(default)]
    pub enabled: bool,
}

impl Default for HookEntryConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Internal hooks block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HooksInternalConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub entries: BTreeMap<String, HookEntryConfig>,
}

impl Default for HooksInternalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            entries: BTreeMap::new(),
        }
    }
}

/// Hooks root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HooksConfig {
    #[serde(default)]
    pub internal: HooksInternalConfig,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            internal: HooksInternalConfig::default(),
        }
    }
}

/// Browser / headless browser settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConfig {
    #[serde(default)]
    pub executable_path: Option<String>,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub default_profile: Option<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            executable_path: None,
            headless: true,
            default_profile: None,
        }
    }
}

/// Single auth profile entry (provider + mode only — no apiKey).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfileEntryConfig {
    pub provider: String,
    #[serde(default)]
    pub mode: String,
}

impl Default for AuthProfileEntryConfig {
    fn default() -> Self {
        Self {
            provider: String::new(),
            mode: String::new(),
        }
    }
}

/// Auth profiles map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfilesConfig {
    #[serde(default)]
    pub profiles: BTreeMap<String, AuthProfileEntryConfig>,
}

impl Default for AuthProfilesConfig {
    fn default() -> Self {
        Self {
            profiles: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// SystemConfigData
// ---------------------------------------------------------------------------

/// Root system configuration.
///
/// Represents the union of all "system" fields in openclaw.json:
/// wizard, update, meta, messages, commands, session, cron, hooks,
/// browser, and auth (profiles only).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemConfigData {
    #[serde(default)]
    pub wizard: Option<WizardConfig>,
    #[serde(default)]
    pub update: Option<UpdateConfig>,
    #[serde(default)]
    pub meta: Option<MetaConfig>,
    #[serde(default)]
    pub messages: Option<MessagesConfig>,
    #[serde(default)]
    pub commands: Option<CommandsConfig>,
    #[serde(default)]
    pub session: Option<SessionConfig>,
    #[serde(default)]
    pub cron: Option<CronConfig>,
    #[serde(default)]
    pub hooks: Option<HooksConfig>,
    #[serde(default)]
    pub browser: Option<BrowserConfig>,
    #[serde(default)]
    pub auth: Option<AuthProfilesConfig>,
}

impl Default for SystemConfigData {
    fn default() -> Self {
        Self {
            wizard: None,
            update: None,
            meta: None,
            messages: None,
            commands: None,
            session: None,
            cron: None,
            hooks: None,
            browser: None,
            auth: None,
        }
    }
}

impl SystemConfigData {
    /// Load from a file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }

    /// Parse from a JSON string.
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let config: SystemConfigData = serde_json::from_str(content)?;
        Ok(config)
    }
}

impl ConfigProvider for SystemConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(ref session) = self.session {
            let valid_modes = ["enforce", "warn", "off"];
            if !valid_modes.contains(&session.maintenance.mode.as_str()) {
                return Err(ConfigError::ValueError {
                    field: "session.maintenance.mode".to_string(),
                    message: format!(
                        "mode must be one of {:?}, got '{}'",
                        valid_modes, session.maintenance.mode
                    ),
                });
            }
        }
        if let Some(ref session) = self.session {
            let valid_scopes = [
                "per-account-channel-peer",
                "per-channel-peer",
                "per-peer",
                "main",
            ];
            if !valid_scopes.contains(&session.dm_scope.as_str()) {
                return Err(ConfigError::ValueError {
                    field: "session.dmScope".to_string(),
                    message: format!(
                        "dmScope must be one of {:?}, got '{}'",
                        valid_scopes, session.dm_scope
                    ),
                });
            }
        }
        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "openclaw.json (system section)"
    }

    fn is_default(&self) -> bool {
        self.wizard.is_none()
            && self.update.as_ref().map_or(true, |u| u.check_on_start)
            && self.meta.is_none()
            && self
                .messages
                .as_ref()
                .map_or(true, |m| m.ack_reaction_scope.is_none())
            && self
                .commands
                .as_ref()
                .map_or(true, |c| c == &CommandsConfig::default())
            && self
                .session
                .as_ref()
                .map_or(true, |s| s == &SessionConfig::default())
            && self.cron.as_ref().map_or(true, |c| c.enabled)
            && self
                .hooks
                .as_ref()
                .map_or(true, |h| h == &HooksConfig::default())
            && self
                .browser
                .as_ref()
                .map_or(true, |b| b == &BrowserConfig::default())
            && self.auth.is_none()
    }
}
