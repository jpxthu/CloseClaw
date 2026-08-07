use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Five-level severity enum for debug log events.
///
/// Severity increases from left to right:
/// `Trace` → `Debug` → `Info` → `Warn` → `Error`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Complete content — full message bodies, request/response payloads.
    Trace,
    /// Intermediate state — routing decisions, permission checks, filter details.
    Debug,
    /// Key events — message arrival, LLM call start/end, tool execution start/end.
    Info,
    /// Degraded warnings — degraded but functional, needs attention.
    Warn,
    /// Errors — functionality unavailable or impaired, must be manually attended to.
    Error,
}

impl LogLevel {
    /// Return numeric severity for ordering (lower = less severe).
    fn severity(self) -> u8 {
        match self {
            Self::Trace => 0,
            Self::Debug => 1,
            Self::Info => 2,
            Self::Warn => 3,
            Self::Error => 4,
        }
    }
}

impl Ord for LogLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.severity().cmp(&other.severity())
    }
}

impl PartialOrd for LogLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        };
        f.write_str(s)
    }
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            _ => Err(format!("unknown log level: {s}")),
        }
    }
}

#[cfg(test)]
#[path = "level_tests.rs"]
mod level_tests;
