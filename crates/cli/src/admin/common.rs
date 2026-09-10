//! Shared helpers and output structs for CLI admin handlers.

use closeclaw_permission::Effect;
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
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
    pub pid: Option<u32>,
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

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Return the CloseClaw root directory via the platform interface.
///
/// This is a thin wrapper around [`closeclaw_platform::config::root_dir`]
/// that keeps the error type as [`anyhow::Result`] for ergonomic `?`
/// propagation inside CLI admin handlers.
pub(crate) fn config_root() -> anyhow::Result<PathBuf> {
    closeclaw_platform::config::root_dir()
}

/// Return the legacy config root path (non-Result).
///
/// This is a compatibility shim kept temporarily so callers that have not
/// yet migrated to [`config_root`] continue to compile.  It will be
/// removed in Step 1.2.
// TODO(step-1.2): remove once callers use config_root()
#[allow(dead_code)]
pub fn config_dir() -> PathBuf {
    config_root().expect("HOME not set")
}

#[allow(dead_code)]
pub fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        "****".to_string()
    } else {
        format!("{}....{}", &key[..4], &key[key.len() - 4..])
    }
}
