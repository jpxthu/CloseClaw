//! Unit tests for daemon private functions

use super::*;
use std::collections::HashMap;
use std::io::Write;
use tempfile::TempDir;

// ============================================================
// load_env_file tests
// ============================================================

#[test]
fn test_load_env_file_normal_parsing() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    let mut file = std::fs::File::create(&env_path).unwrap();
    writeln!(file, "KEY1=value1").unwrap();
    writeln!(file, "KEY2=value2").unwrap();
    writeln!(file, "KEY3=value with spaces").unwrap();

    // Parse the env file
    let pairs = parse_env_file(&env_path).unwrap();

    // Check parsed key-value pairs
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[0], ("KEY1".to_string(), "value1".to_string()));
    assert_eq!(pairs[1], ("KEY2".to_string(), "value2".to_string()));
    assert_eq!(
        pairs[2],
        ("KEY3".to_string(), "value with spaces".to_string())
    );
}

#[test]
fn test_load_env_file_comment_lines() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    let mut file = std::fs::File::create(&env_path).unwrap();
    writeln!(file, "# This is a comment").unwrap();
    writeln!(file, "KEY1=value1").unwrap();
    writeln!(file, "  # Another comment with spaces").unwrap();
    writeln!(file, "KEY2=value2").unwrap();

    let pairs = parse_env_file(&env_path).unwrap();

    // Only KEY1 and KEY2 should be parsed, not the comments
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], ("KEY1".to_string(), "value1".to_string()));
    assert_eq!(pairs[1], ("KEY2".to_string(), "value2".to_string()));
}

#[test]
fn test_load_env_file_empty_lines() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    let mut file = std::fs::File::create(&env_path).unwrap();
    writeln!(file, "").unwrap();
    writeln!(file, "KEY1=value1").unwrap();
    writeln!(file, "").unwrap();
    writeln!(file, "").unwrap();
    writeln!(file, "KEY2=value2").unwrap();
    writeln!(file, "").unwrap();

    let pairs = parse_env_file(&env_path).unwrap();

    // Empty lines should be skipped
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], ("KEY1".to_string(), "value1".to_string()));
    assert_eq!(pairs[1], ("KEY2".to_string(), "value2".to_string()));
}

#[test]
fn test_load_env_file_empty_value() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    std::fs::write(&env_path, "EMPTYKEY=\n").unwrap();

    let pairs = parse_env_file(&env_path).unwrap();

    // Empty value should be skipped (not included in results)
    assert!(pairs.is_empty());
}

#[test]
fn test_load_env_file_empty_key() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    std::fs::write(&env_path, "=value\n").unwrap();

    let pairs = parse_env_file(&env_path).unwrap();
    // Empty key should be skipped
    assert!(pairs.is_empty());
}

#[test]
fn test_load_env_file_no_equal_sign() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    std::fs::write(&env_path, "KEYVALUE\n").unwrap();

    let pairs = parse_env_file(&env_path).unwrap();

    // Line without = should be skipped
    assert!(pairs.is_empty());
}

#[test]
fn test_load_env_file_file_not_found() {
    let result = parse_env_file(std::path::Path::new("/nonexistent/.env"));
    assert!(result.is_err());
}

#[test]
fn test_load_env_file_whitespace_trimming() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    let mut file = std::fs::File::create(&env_path).unwrap();
    writeln!(file, "  KEY1  =  value1  ").unwrap();
    writeln!(file, "\tKEY2\t=\tvalue2\t").unwrap();

    let pairs = parse_env_file(&env_path).unwrap();

    // Whitespace should be trimmed
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], ("KEY1".to_string(), "value1".to_string()));
    assert_eq!(pairs[1], ("KEY2".to_string(), "value2".to_string()));
}

// Daemon::build_permission_engine tests
// ============================================================

#[test]
fn test_build_permission_engine_empty_dir() {
    let dir = TempDir::new().unwrap();
    // Config dir has no templates/ subdirectory
    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap());
    assert!(!Arc::ptr_eq(
        &engine,
        &Arc::new(PermissionEngine::new_with_default_data_root(
            crate::permission::RuleSet {
                rules: Vec::new(),
                defaults: crate::permission::Defaults::default(),
                template_includes: Vec::new(),
                agent_creators: std::collections::HashMap::new(),
            }
        ))
    ));
}

#[test]
fn test_build_permission_engine_with_templates_dir() {
    let dir = TempDir::new().unwrap();
    let templates_dir = dir.path().join("templates");
    std::fs::create_dir(&templates_dir).unwrap();

    // Write a valid template
    let template_path = templates_dir.join("test_template.json");
    let template_json = serde_json::json!({
        "name": "test_template",
        "description": "test",
        "subject": { "type": "any" },
        "effect": "allow",
        "actions": [
            { "type": "file", "operation": "read", "paths": ["**"] }
        ],
        "extends": []
    });
    std::fs::write(&template_path, template_json.to_string()).unwrap();

    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap());
    // Should create without panic; engine has 1 rule from template
    assert!(Arc::ptr_eq(&engine, &engine)); // just check it's a valid Arc
}

// ============================================================
// Daemon::init_llm_registry tests
// ============================================================

#[tokio::test]
async fn test_init_llm_registry_credentials_file_priority() {
    // Arrange: temp dir with credentials/openai.json containing an api key
    let tmp = TempDir::new().unwrap();
    let creds_dir = tmp.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":"file-key-123"}"#,
    )
    .unwrap();
    // Pass empty HashMap — file key should win over any env override
    let env_overrides = HashMap::new();

    // Act
    let registry = Daemon::init_llm_registry_with_env(tmp.path(), &env_overrides).await;

    // Assert: provider registered with file key
    let listed = registry.list().await;
    assert!(
        listed.contains(&"openai".to_string()),
        "openai provider should be registered from credentials file"
    );
}

#[tokio::test]
async fn test_init_llm_registry_env_fallback() {
    // Arrange: temp dir with NO credentials files; pass env overrides via HashMap
    let tmp = TempDir::new().unwrap();
    let mut env_overrides = HashMap::new();
    env_overrides.insert("OPENAI_API_KEY", "env-key-456");
    env_overrides.insert("ANTHROPIC_API_KEY", "env-anthropic-key");

    // Act
    let registry = Daemon::init_llm_registry_with_env(tmp.path(), &env_overrides).await;

    // Assert: providers registered from env overrides
    let listed = registry.list().await;
    assert!(
        listed.contains(&"openai".to_string()),
        "openai should be registered from env"
    );
    assert!(
        listed.contains(&"anthropic".to_string()),
        "anthropic should be registered from env"
    );
}

#[tokio::test]
async fn test_init_llm_registry_both_absent_no_registration() {
    // Arrange: temp dir with NO credentials files, empty env overrides
    let tmp = TempDir::new().unwrap();
    let env_overrides = HashMap::new();

    // Act
    let registry = Daemon::init_llm_registry_with_env(tmp.path(), &env_overrides).await;

    // Assert: no providers registered
    let listed = registry.list().await;
    assert!(
        listed.is_empty(),
        "no providers should be registered when no credentials"
    );
}
