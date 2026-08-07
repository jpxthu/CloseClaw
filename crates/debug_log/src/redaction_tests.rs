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
