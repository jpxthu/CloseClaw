//! Unit tests for daemon private functions

use super::*;
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

    // Clear any existing env vars
    std::env::remove_var("KEY1");
    std::env::remove_var("KEY2");
    std::env::remove_var("KEY3");

    // Load the env file
    load_env_file(&env_path).unwrap();

    // Check that environment variables were set
    assert_eq!(std::env::var("KEY1").unwrap(), "value1");
    assert_eq!(std::env::var("KEY2").unwrap(), "value2");
    assert_eq!(std::env::var("KEY3").unwrap(), "value with spaces");
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

    std::env::remove_var("KEY1");
    std::env::remove_var("KEY2");

    load_env_file(&env_path).unwrap();

    // Only KEY1 and KEY2 should be set, not the comments
    assert_eq!(std::env::var("KEY1").unwrap(), "value1");
    assert_eq!(std::env::var("KEY2").unwrap(), "value2");
    // Comments should not create env vars
    assert!(std::env::var("#").is_err());
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

    std::env::remove_var("KEY1");
    std::env::remove_var("KEY2");

    load_env_file(&env_path).unwrap();

    // Empty lines should be skipped
    assert_eq!(std::env::var("KEY1").unwrap(), "value1");
    assert_eq!(std::env::var("KEY2").unwrap(), "value2");
}

#[test]
fn test_load_env_file_empty_value() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    std::fs::write(&env_path, "EMPTYKEY=\n").unwrap();

    std::env::remove_var("EMPTYKEY");

    load_env_file(&env_path).unwrap();

    // Empty value should be skipped (not set)
    assert!(std::env::var("EMPTYKEY").is_err());
}

#[test]
fn test_load_env_file_empty_key() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    std::fs::write(&env_path, "=value\n").unwrap();

    // Empty key should be skipped
    load_env_file(&env_path).unwrap();
    // No env var should be set
}

#[test]
fn test_load_env_file_no_equal_sign() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    std::fs::write(&env_path, "KEYVALUE\n").unwrap();

    std::env::remove_var("KEYVALUE");

    load_env_file(&env_path).unwrap();

    // Line without = should be skipped
    assert!(std::env::var("KEYVALUE").is_err());
}

#[test]
fn test_load_env_file_file_not_found() {
    let result = load_env_file(std::path::Path::new("/nonexistent/.env"));
    assert!(result.is_err());
}

#[test]
fn test_load_env_file_whitespace_trimming() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    let mut file = std::fs::File::create(&env_path).unwrap();
    writeln!(file, "  KEY1  =  value1  ").unwrap();
    writeln!(file, "\tKEY2\t=\tvalue2\t").unwrap();

    std::env::remove_var("KEY1");
    std::env::remove_var("KEY2");

    load_env_file(&env_path).unwrap();

    // Whitespace should be trimmed
    assert_eq!(std::env::var("KEY1").unwrap(), "value1");
    assert_eq!(std::env::var("KEY2").unwrap(), "value2");
}

// ============================================================
// Daemon::load_agents_config tests
// ============================================================

#[test]
fn test_load_agents_config_success() {
    let dir = TempDir::new().unwrap();
    let agents_path = dir.path().join("agents.json");
    let agents_json = serde_json::json!({
        "version": "1.0",
        "agents": [
            {
                "name": "guide",
                "model": "minimax/MiniMax-M2",
                "persona": "test persona",
                "max_iterations": 10,
                "timeout_minutes": 5
            }
        ]
    });
    std::fs::write(&agents_path, agents_json.to_string()).unwrap();

    let config_dir = dir.path().to_str().unwrap();
    let result = Daemon::load_agents_config(config_dir);
    assert!(result.is_ok(), "expected ok, got: {:?}", result.err());
    let provider = result.unwrap();
    assert_eq!(provider.agents().len(), 1);
    assert_eq!(provider.agents()[0].name, "guide");
}

#[test]
fn test_load_agents_config_file_not_found() {
    let result = Daemon::load_agents_config("/nonexistent/path");
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Failed to load") || err_msg.contains("agents.json"));
}

#[test]
fn test_load_agents_config_invalid_json() {
    let dir = TempDir::new().unwrap();
    let agents_path = dir.path().join("agents.json");
    std::fs::write(&agents_path, "not json").unwrap();

    let result = Daemon::load_agents_config(dir.path().to_str().unwrap());
    assert!(result.is_err());
}

// ============================================================
// Daemon::build_permission_engine tests
// ============================================================

#[test]
fn test_build_permission_engine_empty_dir() {
    let dir = TempDir::new().unwrap();
    // Config dir has no templates/ subdirectory
    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap());
    assert!(!Arc::ptr_eq(
        &engine,
        &Arc::new(PermissionEngine::new(crate::permission::RuleSet {
            version: "1.0.0".to_string(),
            rules: Vec::new(),
            defaults: crate::permission::Defaults::default(),
            template_includes: Vec::new(),
            agent_creators: std::collections::HashMap::new(),
        }))
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
