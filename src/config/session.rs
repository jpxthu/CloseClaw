//! Session configuration module
//!
//! Provides per-agent per-role session configuration including idle timeout
//! and purge-after settings for the ArchiveSweeper and Daemon integrations.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::providers::ConfigError;
use crate::session::compaction::CompactConfig;
use crate::session::persistence::AgentRole;

#[cfg(test)]
#[path = "session_compact_tests.rs"]
mod session_compact_tests;

/// Default idle time in minutes before a session is considered idle
pub const DEFAULT_IDLE_MINUTES: i64 = 30;
/// Default purge time in minutes after which archived sessions are permanently deleted
pub const DEFAULT_PURGE_AFTER_MINUTES: i64 = 10080; // 7 days
/// Default sweeper interval in seconds
pub const DEFAULT_SWEEPER_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Per-agent per-role session configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerAgentSessionConfig {
    /// Idle timeout in minutes
    pub idle_minutes: i64,
    /// Purge-after timeout in minutes (0 = never purge)
    pub purge_after_minutes: i64,
}

impl PerAgentSessionConfig {
    /// Create a new PerAgentSessionConfig with the given values
    pub fn new(idle_minutes: i64, purge_after_minutes: i64) -> Self {
        Self {
            idle_minutes,
            purge_after_minutes,
        }
    }
}

impl Default for PerAgentSessionConfig {
    fn default() -> Self {
        Self {
            idle_minutes: DEFAULT_IDLE_MINUTES,
            purge_after_minutes: DEFAULT_PURGE_AFTER_MINUTES,
        }
    }
}

/// Hard-coded fallback defaults (lowest priority)
fn hardcoded_config(_role: AgentRole) -> PerAgentSessionConfig {
    PerAgentSessionConfig::default()
}

/// Session configuration container
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    /// Default per-role config for all agents
    pub defaults: BTreeMap<AgentRole, PerAgentSessionConfig>,
    /// Per-agent overrides (agent_id -> role -> config)
    #[serde(default)]
    pub agents: BTreeMap<String, BTreeMap<AgentRole, PerAgentSessionConfig>>,
    /// Sweeper interval in seconds (default: 5 minutes)
    #[serde(default = "default_sweeper_interval")]
    pub sweeper_interval_secs: u64,
    /// Compaction configuration (optional, falls back to CompactConfig::default())
    #[serde(default)]
    pub compact: Option<CompactConfig>,
}

fn default_sweeper_interval() -> u64 {
    DEFAULT_SWEEPER_INTERVAL_SECS
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            defaults: BTreeMap::new(),
            agents: BTreeMap::new(),
            sweeper_interval_secs: DEFAULT_SWEEPER_INTERVAL_SECS,
            compact: None,
        }
    }
}

/// Session configuration provider trait
pub trait SessionConfigProvider: Send + Sync {
    /// Get session config for a specific agent and role
    fn session_config_for(&self, agent_id: &str, role: AgentRole) -> PerAgentSessionConfig;

    /// Get sweeper interval in seconds
    fn sweeper_interval_secs(&self) -> u64;

    /// List all agent IDs that have per-agent overrides
    fn list_agents(&self) -> Vec<String>;

    /// Get compaction configuration
    fn compact_config(&self) -> CompactConfig;
}

/// JSON-based session configuration provider
#[derive(Debug, Clone)]
pub struct JsonSessionConfigProvider {
    /// Parsed session config (None means file was absent, using hardcoded defaults)
    config: Option<SessionConfig>,
    /// Path to the config file (for error messages)
    _path: String,
}

impl JsonSessionConfigProvider {
    /// Create a new provider from a JSON file path.
    ///
    /// - File does not exist → `warn!` + hardcoded defaults (no error)
    /// - JSON parse error → `Err(ConfigError::SchemaError(...))`
    /// - Value validation errors (negative values) → `Err(ConfigError::ValueError(...))`
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path_str = path.as_ref().display().to_string();

        let config = match fs::read_to_string(path.as_ref()) {
            Ok(content) => {
                let config: SessionConfig = serde_json::from_str(&content)
                    .map_err(|e| ConfigError::SchemaError(format!("invalid JSON: {}", e)))?;
                Some(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                warn!(
                    path = %path_str,
                    "session config file not found, using hardcoded defaults"
                );
                None
            }
            Err(e) => {
                // Unexpected IO error (not just missing file)
                return Err(ConfigError::IoError(e));
            }
        };

        let provider = Self {
            config,
            _path: path_str,
        };
        provider.validate()?;
        Ok(provider)
    }

    /// Validate the loaded config (no-op if file was absent).
    ///
    /// Returns `Ok(())` if config is absent (using defaults) or valid.
    /// Returns `Err(ConfigError::ValueError)` if any field value is invalid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(ref config) = self.config {
            // Validate defaults
            for (role, cfg) in &config.defaults {
                if cfg.idle_minutes < 0 {
                    return Err(ConfigError::ValueError {
                        field: format!("defaults.{:?}.idle_minutes", role),
                        message: format!("idle_minutes must be >= 0, got {}", cfg.idle_minutes),
                    });
                }
                if cfg.purge_after_minutes < 0 {
                    return Err(ConfigError::ValueError {
                        field: format!("defaults.{:?}.purge_after_minutes", role),
                        message: format!(
                            "purge_after_minutes must be >= 0, got {}",
                            cfg.purge_after_minutes
                        ),
                    });
                }
            }

            // Validate per-agent overrides
            for (agent_id, roles) in &config.agents {
                for (role, cfg) in roles {
                    if cfg.idle_minutes < 0 {
                        return Err(ConfigError::ValueError {
                            field: format!("agents.{}.{:?}.idle_minutes", agent_id, role),
                            message: format!("idle_minutes must be >= 0, got {}", cfg.idle_minutes),
                        });
                    }
                    if cfg.purge_after_minutes < 0 {
                        return Err(ConfigError::ValueError {
                            field: format!("agents.{}.{:?}.purge_after_minutes", agent_id, role),
                            message: format!(
                                "purge_after_minutes must be >= 0, got {}",
                                cfg.purge_after_minutes
                            ),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

impl SessionConfigProvider for JsonSessionConfigProvider {
    /// Get session config using fallback chain:
    /// per-agent override → defaults → hardcoded defaults
    fn session_config_for(&self, agent_id: &str, role: AgentRole) -> PerAgentSessionConfig {
        // 1. Per-agent override (highest priority)
        if let Some(ref config) = self.config {
            if let Some(roles) = config.agents.get(agent_id) {
                if let Some(cfg) = roles.get(&role) {
                    return cfg.clone();
                }
            }

            // 2. Defaults (middle priority)
            if let Some(default_cfg) = config.defaults.get(&role) {
                return default_cfg.clone();
            }
        }

        // 3. Hardcoded defaults (lowest priority)
        hardcoded_config(role)
    }

    fn sweeper_interval_secs(&self) -> u64 {
        self.config
            .as_ref()
            .map(|c| c.sweeper_interval_secs)
            .unwrap_or(DEFAULT_SWEEPER_INTERVAL_SECS)
    }

    fn list_agents(&self) -> Vec<String> {
        self.config
            .as_ref()
            .map(|c| c.agents.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn compact_config(&self) -> CompactConfig {
        self.config
            .as_ref()
            .and_then(|c| c.compact.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    /// Minimal valid session config JSON with given defaults and agents.
    fn valid_config_json(defaults: &str, agents: &str, sweeper_interval_secs: u64) -> String {
        format!(
            r#"{{"defaults":{},"agents":{},"sweeperIntervalSecs":{}}}"#,
            defaults, agents, sweeper_interval_secs
        )
    }

    /// Write JSON content to a temp file and return its path.
    fn write_temp_json(content: &str) -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("session_config.json");
        std::fs::write(&path, content).unwrap();
        (temp, path)
    }

    // -------------------------------------------------------------------------
    // Test: file exists and parses correctly
    // -------------------------------------------------------------------------

    #[test]
    fn test_new_parses_valid_file() {
        let json = valid_config_json(
            r#"{"mainAgent":{"idleMinutes":10,"purgeAfterMinutes":60}}"#,
            "{}",
            600,
        );
        let (_temp, path) = write_temp_json(&json);

        let provider = JsonSessionConfigProvider::new(&path).unwrap();
        let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);

        assert_eq!(cfg.idle_minutes, 10);
        assert_eq!(cfg.purge_after_minutes, 60);
        assert_eq!(provider.sweeper_interval_secs(), 600);
    }

    // -------------------------------------------------------------------------
    // Test: file does not exist → warn + hardcoded defaults (no panic)
    // -------------------------------------------------------------------------

    #[test]
    fn test_new_missing_file_uses_hardcoded_defaults() {
        let temp = TempDir::new().unwrap();
        let nonexistent = temp.path().join("nonexistent.json");

        // Must not panic; must return Ok with hardcoded defaults
        let provider = JsonSessionConfigProvider::new(&nonexistent).unwrap();

        let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);
        assert_eq!(cfg.idle_minutes, DEFAULT_IDLE_MINUTES);
        assert_eq!(cfg.purge_after_minutes, DEFAULT_PURGE_AFTER_MINUTES);
        assert_eq!(
            provider.sweeper_interval_secs(),
            DEFAULT_SWEEPER_INTERVAL_SECS
        );
    }

    // -------------------------------------------------------------------------
    // Test: malformed JSON → Err(ConfigError::SchemaError)
    // -------------------------------------------------------------------------

    #[test]
    fn test_new_malformed_json_returns_schema_error() {
        let (_temp, path) = write_temp_json("not valid json at all {{{");

        let err = JsonSessionConfigProvider::new(&path).unwrap_err();
        let err_str = err.to_string();
        // The error message should mention schema/parsing failure
        assert!(
            err_str.contains("Schema") || err_str.contains("invalid JSON"),
            "expected SchemaError, got: {}",
            err_str
        );
    }

    // -------------------------------------------------------------------------
    // Test: idle_minutes < 0 → Err(ConfigError::ValueError)
    // -------------------------------------------------------------------------

    #[test]
    fn test_new_negative_idle_minutes_returns_value_error() {
        let json = valid_config_json(
            r#"{"mainAgent":{"idleMinutes":-5,"purgeAfterMinutes":60}}"#,
            "{}",
            300,
        );
        let (_temp, path) = write_temp_json(&json);

        let err = JsonSessionConfigProvider::new(&path).unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("ValueError") || err_str.contains("idle_minutes"),
            "expected ValueError for negative idle_minutes, got: {}",
            err_str
        );
    }

    // -------------------------------------------------------------------------
    // Test: purge_after_minutes < 0 → Err(ConfigError::ValueError)
    // -------------------------------------------------------------------------

    #[test]
    fn test_new_negative_purge_after_minutes_returns_value_error() {
        let json = valid_config_json(
            r#"{"mainAgent":{"idleMinutes":10,"purgeAfterMinutes":-1}}"#,
            "{}",
            300,
        );
        let (_temp, path) = write_temp_json(&json);

        let err = JsonSessionConfigProvider::new(&path).unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("ValueError") || err_str.contains("purge_after_minutes"),
            "expected ValueError for negative purge_after_minutes, got: {}",
            err_str
        );
    }

    // -------------------------------------------------------------------------
    // Test: session_config_for unknown agent → fallback to defaults
    // -------------------------------------------------------------------------

    #[test]
    fn test_session_config_for_unknown_agent_falls_back_to_defaults() {
        let json = valid_config_json(
            r#"{"mainAgent":{"idleMinutes":99,"purgeAfterMinutes":999}}"#,
            "{}",
            300,
        );
        let (_temp, path) = write_temp_json(&json);

        let provider = JsonSessionConfigProvider::new(&path).unwrap();
        let cfg = provider.session_config_for("unknown-agent", AgentRole::MainAgent);

        // Falls back to the defaults section
        assert_eq!(cfg.idle_minutes, 99);
        assert_eq!(cfg.purge_after_minutes, 999);
    }

    // -------------------------------------------------------------------------
    // Test: sweeper_interval_secs returns correct value
    // -------------------------------------------------------------------------

    #[test]
    fn test_sweeper_interval_secs() {
        let json = valid_config_json("{}", "{}", 720);
        let (_temp, path) = write_temp_json(&json);

        let provider = JsonSessionConfigProvider::new(&path).unwrap();
        assert_eq!(provider.sweeper_interval_secs(), 720);
    }

    // -------------------------------------------------------------------------
    // Test: purge_after_minutes = 0 is valid (never purge)
    // -------------------------------------------------------------------------

    #[test]
    fn test_purge_after_minutes_zero_is_valid() {
        let json = valid_config_json(
            r#"{"mainAgent":{"idleMinutes":10,"purgeAfterMinutes":0}}"#,
            "{}",
            300,
        );
        let (_temp, path) = write_temp_json(&json);

        let provider = JsonSessionConfigProvider::new(&path).unwrap();
        let cfg = provider.session_config_for("any", AgentRole::MainAgent);
        assert_eq!(cfg.purge_after_minutes, 0);
    }

    // -------------------------------------------------------------------------
    // Test: purge_after_minutes = 10080 (7 days) default is correct
    // -------------------------------------------------------------------------

    #[test]
    fn test_default_purge_after_minutes_is_10080() {
        assert_eq!(DEFAULT_PURGE_AFTER_MINUTES, 10080); // 7 days in minutes

        // When no config file exists, hardcoded default is 10080
        let temp = TempDir::new().unwrap();
        let nonexistent = temp.path().join("nonexistent.json");
        let provider = JsonSessionConfigProvider::new(&nonexistent).unwrap();
        let cfg = provider.session_config_for("any", AgentRole::MainAgent);
        assert_eq!(cfg.purge_after_minutes, 10080);
    }

    // -------------------------------------------------------------------------
    // Test: list_agents returns all configured agent IDs
    // -------------------------------------------------------------------------

    #[test]
    fn test_list_agents_returns_all_configured_agent_ids() {
        // Two agents with per-agent overrides
        let json = valid_config_json(
            "{}",
            r#"{
                "agent-alpha":{"mainAgent":{"idleMinutes":1,"purgeAfterMinutes":1}},
                "agent-beta":{"subAgent":{"idleMinutes":2,"purgeAfterMinutes":2}}
            }"#,
            300,
        );
        let (_temp, path) = write_temp_json(&json);

        let provider = JsonSessionConfigProvider::new(&path).unwrap();
        let agents = provider.list_agents();

        assert_eq!(agents.len(), 2);
        assert!(agents.contains(&"agent-alpha".to_string()));
        assert!(agents.contains(&"agent-beta".to_string()));
    }

    #[test]
    fn test_list_agents_empty_when_no_config_file() {
        let temp = TempDir::new().unwrap();
        let nonexistent = temp.path().join("nonexistent.json");
        let provider = JsonSessionConfigProvider::new(&nonexistent).unwrap();
        assert!(provider.list_agents().is_empty());
    }
}
