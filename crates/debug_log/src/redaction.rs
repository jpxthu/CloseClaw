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
                    let dominated = self.field_matches(&key);
                    if dominated {
                        if let Some(replacement) = self.replacement_for(&key) {
                            map.insert(key, serde_json::Value::String(replacement));
                        }
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

    /// Check whether any pattern matches the given field name.
    fn field_matches(&self, field: &str) -> bool {
        self.patterns.iter().any(|p| match p.match_type {
            PatternMatch::Exact => p.field.eq_ignore_ascii_case(field),
            PatternMatch::Prefix => field
                .to_ascii_lowercase()
                .starts_with(&p.field.to_ascii_lowercase()),
        })
    }

    /// Return the replacement string for the first matching pattern.
    fn replacement_for(&self, field: &str) -> Option<String> {
        self.patterns
            .iter()
            .find(|p| match p.match_type {
                PatternMatch::Exact => p.field.eq_ignore_ascii_case(field),
                PatternMatch::Prefix => field
                    .to_ascii_lowercase()
                    .starts_with(&p.field.to_ascii_lowercase()),
            })
            .map(|p| p.replacement.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_exact_match_redacts() {
        let engine = RedactionEngine::new(vec![RedactionPattern {
            field: "api_key".into(),
            match_type: PatternMatch::Exact,
            replacement: "[REDACTED]".into(),
        }]);
        let mut payload = json!({"api_key": "sk-secret123", "user": "alice"});
        engine.redact(&mut payload);
        assert_eq!(payload["api_key"], "[REDACTED]");
        assert_eq!(payload["user"], "alice");
    }

    #[test]
    fn test_case_insensitive_exact() {
        let engine = RedactionEngine::new(vec![RedactionPattern {
            field: "token".into(),
            match_type: PatternMatch::Exact,
            replacement: "[REDACTED]".into(),
        }]);
        let mut payload = json!({"TOKEN": "abc", "Token": "def", "token": "ghi"});
        engine.redact(&mut payload);
        assert_eq!(payload["TOKEN"], "[REDACTED]");
        assert_eq!(payload["Token"], "[REDACTED]");
        assert_eq!(payload["token"], "[REDACTED]");
    }

    #[test]
    fn test_prefix_match_redacts() {
        let engine = RedactionEngine::new(vec![RedactionPattern {
            field: "secret".into(),
            match_type: PatternMatch::Prefix,
            replacement: "[REDACTED]".into(),
        }]);
        let mut payload = json!({"secret_key": "abc", "secret_value": "def", "user": "alice"});
        engine.redact(&mut payload);
        assert_eq!(payload["secret_key"], "[REDACTED]");
        assert_eq!(payload["secret_value"], "[REDACTED]");
        assert_eq!(payload["user"], "alice");
    }

    #[test]
    fn test_nested_json_redaction() {
        let engine = RedactionEngine::new(vec![RedactionPattern {
            field: "password".into(),
            match_type: PatternMatch::Exact,
            replacement: "[REDACTED]".into(),
        }]);
        let mut payload = json!({
            "user": {
                "name": "alice",
                "credentials": {
                    "password": "hunter2",
                    "pin": "1234"
                }
            }
        });
        engine.redact(&mut payload);
        assert_eq!(payload["user"]["name"], "alice");
        assert_eq!(payload["user"]["credentials"]["password"], "[REDACTED]");
        assert_eq!(payload["user"]["credentials"]["pin"], "1234");
    }

    #[test]
    fn test_array_redaction() {
        let engine = RedactionEngine::new(vec![RedactionPattern {
            field: "token".into(),
            match_type: PatternMatch::Exact,
            replacement: "[REDACTED]".into(),
        }]);
        let mut payload = json!({
            "items": [
                {"token": "abc", "id": 1},
                {"token": "def", "id": 2}
            ]
        });
        engine.redact(&mut payload);
        assert_eq!(payload["items"][0]["token"], "[REDACTED]");
        assert_eq!(payload["items"][0]["id"], 1);
        assert_eq!(payload["items"][1]["token"], "[REDACTED]");
    }

    #[test]
    fn test_no_match_unchanged() {
        let engine = RedactionEngine::new(vec![RedactionPattern {
            field: "api_key".into(),
            match_type: PatternMatch::Exact,
            replacement: "[REDACTED]".into(),
        }]);
        let mut payload = json!({"name": "alice", "count": 42});
        engine.redact(&mut payload);
        assert_eq!(payload["name"], "alice");
        assert_eq!(payload["count"], 42);
    }

    #[test]
    fn test_empty_payload() {
        let engine = RedactionEngine::new(vec![RedactionPattern {
            field: "api_key".into(),
            match_type: PatternMatch::Exact,
            replacement: "[REDACTED]".into(),
        }]);
        let mut payload = json!({});
        engine.redact(&mut payload);
        assert_eq!(payload, json!({}));
    }

    #[test]
    fn test_no_patterns_noop() {
        let engine = RedactionEngine::empty();
        let mut payload = json!({"api_key": "sk-secret"});
        engine.redact(&mut payload);
        assert_eq!(payload["api_key"], "sk-secret");
    }

    #[test]
    fn test_multiple_patterns() {
        let engine = RedactionEngine::new(vec![
            RedactionPattern {
                field: "api_key".into(),
                match_type: PatternMatch::Exact,
                replacement: "[REDACTED]".into(),
            },
            RedactionPattern {
                field: "secret".into(),
                match_type: PatternMatch::Prefix,
                replacement: "[HIDDEN]".into(),
            },
        ]);
        let mut payload = json!({
            "api_key": "sk-abc",
            "secret_token": "xyz",
            "name": "alice"
        });
        engine.redact(&mut payload);
        assert_eq!(payload["api_key"], "[REDACTED]");
        assert_eq!(payload["secret_token"], "[HIDDEN]");
        assert_eq!(payload["name"], "alice");
    }

    #[test]
    fn test_prefix_case_insensitive() {
        let engine = RedactionEngine::new(vec![RedactionPattern {
            field: "auth".into(),
            match_type: PatternMatch::Prefix,
            replacement: "[REDACTED]".into(),
        }]);
        let mut payload = json!({"Authorization": "Bearer token", "AUTH_KEY": "abc"});
        engine.redact(&mut payload);
        assert_eq!(payload["Authorization"], "[REDACTED]");
        assert_eq!(payload["AUTH_KEY"], "[REDACTED]");
    }
}
