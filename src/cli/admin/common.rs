//! Shared helpers and output structs for CLI admin handlers.

use crate::permission::Effect;
use serde::Serialize;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// JSON output helpers
// ---------------------------------------------------------------------------

/// Print a serializable value as pretty-printed JSON and exit.
pub(crate) fn json_output<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{}", s),
        Err(e) => {
            eprintln!("JSON serialization error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Return a JSON error value suitable for propagation with `?`.
pub(crate) fn json_error(message: &str) -> anyhow::Error {
    #[derive(Serialize)]
    struct ErrorOutput<'a> {
        error: &'a str,
    }
    json_output(&ErrorOutput { error: message });
    anyhow::anyhow!(message.to_string())
}

/// Convert a permission [`Effect`] to its string representation.
pub(crate) fn effect_to_str(effect: Effect) -> &'static str {
    match effect {
        Effect::Allow => "allow",
        Effect::Deny => "deny",
    }
}

// ---------------------------------------------------------------------------
// JSON output structs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ConfigValidateOutput {
    pub file: String,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct ConfigListFile {
    pub name: String,
    pub version: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct ConfigListOutput {
    pub files: Vec<ConfigListFile>,
}

#[derive(Serialize)]
pub struct RuleCheckOutput {
    pub rule_name: String,
    pub valid: bool,
}

#[derive(Serialize)]
pub struct RuleListEntry {
    pub name: String,
    pub subject: String,
    pub effect: String,
    pub action_count: usize,
}

#[derive(Serialize)]
pub struct RuleListOutput {
    pub rules: Vec<RuleListEntry>,
}

#[derive(Serialize)]
pub struct StopOutput {
    pub pid: u32,
    pub signal: String,
    pub stopped: bool,
}

#[derive(Serialize)]
pub struct RunOutput {
    pub pid: u32,
    pub config_dir: String,
    pub started: bool,
}

#[derive(Serialize)]
pub struct AgentCreateOutput {
    pub status: &'static str,
    pub name: String,
}

#[derive(Serialize)]
pub struct SkillInstallOutput {
    pub status: &'static str,
    pub name: String,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        "****".to_string()
    } else {
        format!("{}....{}", &key[..4], &key[key.len() - 4..])
    }
}

pub fn pid_file_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".closeclaw").join("daemon.pid")
}

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    config_dir_for(home)
}

pub fn config_dir_for(home: impl AsRef<std::path::Path>) -> PathBuf {
    PathBuf::from(home.as_ref()).join(".closeclaw")
}
