//! Bootstrap file collection mode.

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
