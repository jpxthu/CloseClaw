//! Bootstrap file collection mode.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Bootstrap file collection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BootstrapMode {
    /// Runtime-required identity/tool files, minimal token consumption.
    Minimal,
    /// Full set, including files that need persistent context/memory.
    Full,
}

impl fmt::Display for BootstrapMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootstrapMode::Minimal => write!(f, "minimal"),
            BootstrapMode::Full => write!(f, "full"),
        }
    }
}
