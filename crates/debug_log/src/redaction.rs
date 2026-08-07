use serde::{Deserialize, Serialize};

/// Match strategy for a redaction pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternMatch {
    /// Exact field name match (case-insensitive).
    Exact,
    /// Prefix match on field name (case-insensitive).
    Prefix,
}

/// A rule describing how to identify sensitive fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionPattern {
    /// The field name or prefix to match (e.g. "api_key", "token").
    pub field: String,
    /// Match strategy.
    #[serde(default = "default_exact")]
    pub match_type: PatternMatch,
    /// Replacement value (defaults to `[REDACTED]`).
    #[serde(default = "default_replacement")]
    pub replacement: String,
}

fn default_exact() -> PatternMatch {
    PatternMatch::Exact
}

fn default_replacement() -> String {
    "[REDACTED]".to_string()
}

/// Credential redaction engine that scans JSON payloads for sensitive
/// fields and replaces their values with a redaction marker.
///
/// Sensitive data is never recorded in plaintext at any log level.
#[derive(Debug, Clone)]
pub struct RedactionEngine {
    patterns: Vec<RedactionPattern>,
}

impl RedactionEngine {
    /// Create a new engine with the given patterns.
    pub fn new(patterns: Vec<RedactionPattern>) -> Self {
        Self { patterns }
    }

    /// Create an engine with no patterns (no-op redaction).
    pub fn empty() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

    /// Recursively scan `payload` and replace values of matching fields
    /// with the configured replacement string.
    pub fn redact(&self, payload: &mut serde_json::Value) {
        match payload {
            serde_json::Value::Object(map) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                for key in keys {
                    if let Some(pattern) = self.find_matching_pattern(&key) {
                        map.insert(key, serde_json::Value::String(pattern.replacement.clone()));
                    } else if let Some(val) = map.get_mut(&key) {
                        self.redact(val);
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr.iter_mut() {
                    self.redact(item);
                }
            }
            _ => {}
        }
    }

    /// Find the first pattern that matches the given field name.
    fn find_matching_pattern(&self, field: &str) -> Option<&RedactionPattern> {
        self.patterns.iter().find(|p| match p.match_type {
            PatternMatch::Exact => p.field.eq_ignore_ascii_case(field),
            PatternMatch::Prefix => field
                .to_ascii_lowercase()
                .starts_with(&p.field.to_ascii_lowercase()),
        })
    }
}

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod redaction_tests;
