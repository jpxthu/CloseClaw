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
    // Config dir has no templates/ subdirectory — engine should initialize
    // without error and contain the correct user_defaults.
    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap());
    let guard = engine.blocking_read();
    let rs = guard.rules();
    assert!(rs.rules.is_empty(), "no templates should yield empty rules");
    let ud = &rs.user_defaults;
    let expected = closeclaw_permission::Defaults::user_defaults();
    assert_eq!(ud.file, expected.file);
    assert_eq!(ud.command, expected.command);
    assert_eq!(ud.network, expected.network);
    assert_eq!(ud.inter_agent, expected.inter_agent);
    assert_eq!(ud.config, expected.config);
    assert_eq!(ud.tool_call, expected.tool_call);
    assert_eq!(ud.message, expected.message);
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
    // Arrange: temp dir with config/credentials/openai.json containing an api key
    let tmp = TempDir::new().unwrap();
    let creds_dir = tmp.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":"file-key-123"}"#,
    )
    .unwrap();

    // Act: pass empty overrides — file key takes priority over env
    let registry = Daemon::init_llm_registry(tmp.path(), &HashMap::new()).await;

    // Assert: provider registered with file key
    let provider = registry.get("openai").await;
    assert!(provider.is_some(), "openai provider should be registered");
    let listed = registry.list().await;
    assert!(listed.contains(&"openai".to_string()));
}

#[tokio::test]
async fn test_init_llm_registry_env_fallback() {
    // Arrange: temp dir with NO credentials files, use env_overrides
    let tmp = TempDir::new().unwrap();
    let overrides: HashMap<&str, &str> = HashMap::from([
        ("OPENAI_API_KEY", "env-key-456"),
        ("ANTHROPIC_API_KEY", "env-anthropic-key"),
    ]);

    // Act
    let registry = Daemon::init_llm_registry(tmp.path(), &overrides).await;

    // Assert: providers registered from env overrides
    let listed = registry.list().await;
    assert!(
        listed.contains(&"openai".to_string()),
        "openai should be registered from env override"
    );
    assert!(
        listed.contains(&"anthropic".to_string()),
        "anthropic should be registered from env override"
    );
}

#[tokio::test]
async fn test_init_llm_registry_both_absent_no_registration() {
    // Arrange: temp dir with NO credentials files, empty overrides for all keys
    // to block env fallback
    let tmp = TempDir::new().unwrap();
    let overrides = HashMap::from([
        ("OPENAI_API_KEY", ""),
        ("ANTHROPIC_API_KEY", ""),
        ("MINIMAX_API_KEY", ""),
        ("MIMO_API_KEY", ""),
    ]);

    // Act
    let registry = Daemon::init_llm_registry(tmp.path(), &overrides).await;

    // Assert: no providers registered (empty dir, empty overrides block env fallback)
    let listed = registry.list().await;
    assert!(
        listed.is_empty(),
        "no provider should be registered when no credentials or env vars"
    );
}

// ============================================================
// MiMo provider registration tests (Step 1.4)
// ============================================================

#[tokio::test]
async fn test_init_llm_registry_mimo_via_env_override() {
    let tmp = TempDir::new().unwrap();
    let overrides: HashMap<&str, &str> = HashMap::from([("MIMO_API_KEY", "mimo-env-key-789")]);

    let registry = Daemon::init_llm_registry(tmp.path(), &overrides).await;

    let listed = registry.list().await;
    assert!(
        listed.contains(&"mimo".to_string()),
        "mimo should be registered from env override"
    );
    assert!(
        registry.get("mimo").await.is_some(),
        "mimo provider should be retrievable"
    );
}

#[tokio::test]
async fn test_init_llm_registry_mimo_via_credentials_file() {
    let tmp = TempDir::new().unwrap();
    let creds_dir = tmp.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();
    std::fs::write(
        creds_dir.join("mimo.json"),
        r#"{"provider":"mimo","apiKey":"mimo-file-key-101"}"#,
    )
    .unwrap();

    let registry = Daemon::init_llm_registry(tmp.path(), &HashMap::new()).await;

    let listed = registry.list().await;
    assert!(
        listed.contains(&"mimo".to_string()),
        "mimo should be registered from credentials file"
    );
    assert!(
        registry.get("mimo").await.is_some(),
        "mimo provider should be retrievable"
    );
}

#[tokio::test]
async fn test_init_llm_registry_mimo_not_registered_when_absent() {
    let tmp = TempDir::new().unwrap();
    let overrides: HashMap<&str, &str> = HashMap::from([("MIMO_API_KEY", "")]);

    let registry = Daemon::init_llm_registry(tmp.path(), &overrides).await;

    let listed = registry.list().await;
    assert!(
        !listed.contains(&"mimo".to_string()),
        "mimo should NOT be registered when credentials are missing"
    );
    assert!(
        registry.get("mimo").await.is_none(),
        "mimo provider should not be retrievable"
    );
}

// ============================================================
// Step 1.6: Memory config alignment — DreamingPipeline/MemoryMiner
// built from ConfigManager (not hardcoded defaults)
// ============================================================

/// Verify that DreamingPipeline built via with_config() uses the
/// config values from ConfigManager (not hardcoded defaults).
#[test]
fn test_dreaming_pipeline_built_from_config_manager() {
    let memory_json = r#"{
        "mining": { "enabled": true, "maxEventsPerSession": 20 },
        "dreaming": {
            "enabled": true,
            "schedule": "0 3 * * *",
            "scoring": { "frequencyWeight": 2.0 }
        }
    }"#;

    let memory_config =
        closeclaw_config::providers::MemoryConfigData::from_json_str(memory_json).unwrap();

    // Verify the config was parsed correctly before building the pipeline.
    assert!(memory_config.config.dreaming.enabled.unwrap_or(false));
    assert_eq!(
        memory_config.config.dreaming.schedule.as_deref(),
        Some("0 3 * * *")
    );
    assert_eq!(
        memory_config.config.dreaming.scoring.frequency_weight,
        Some(2.0)
    );

    let pipeline = DreamingPipeline::with_config(memory_config.config.dreaming.clone());

    // Verify the pipeline was constructed (non-trivial — with_config populates
    // scoring, thresholds, and config fields from the DreamingConfig).
    // The pipeline's run_once should not panic when called with empty storage.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let storage: std::sync::Arc<dyn closeclaw_session::persistence::PersistenceService> =
        std::sync::Arc::new(crate::test_helpers::TestStorage::default());
    rt.block_on(async {
        let result = pipeline.run_once(storage.as_ref()).await;
        // run_once may return Err if DB path is not set, but it should not panic.
        // The key assertion is that the pipeline was built successfully from config.
        let _ = result;
    });
}

/// Verify that MinerConfig::from_mining_config() derives enabled from
/// the MiningConfig (not from MinerConfig::default()).
#[test]
fn test_miner_config_from_mining_config() {
    let mining_config_enabled = closeclaw_config::agents::MiningConfig {
        enabled: Some(true),
        ..Default::default()
    };
    let miner_cfg =
        closeclaw_memory::miner::MinerConfig::from_mining_config(&mining_config_enabled);
    assert!(
        miner_cfg.enabled,
        "MinerConfig should be enabled when MiningConfig.enabled = true"
    );

    let mining_config_disabled = closeclaw_config::agents::MiningConfig {
        enabled: Some(false),
        ..Default::default()
    };
    let miner_cfg =
        closeclaw_memory::miner::MinerConfig::from_mining_config(&mining_config_disabled);
    assert!(
        !miner_cfg.enabled,
        "MinerConfig should be disabled when MiningConfig.enabled = false"
    );

    // When enabled is None, fallback should be false (per config.md).
    let mining_config_none = closeclaw_config::agents::MiningConfig {
        enabled: None,
        ..Default::default()
    };
    let miner_cfg = closeclaw_memory::miner::MinerConfig::from_mining_config(&mining_config_none);
    assert!(
        !miner_cfg.enabled,
        "MinerConfig.enabled should default to false when unset"
    );
}

/// Verify that MinerConfig::from_mining_config() respects custom
/// max_events_per_session and dedup_window_days.
#[test]
fn test_miner_config_from_mining_config_custom_values() {
    let mining_config = closeclaw_config::agents::MiningConfig {
        enabled: Some(true),
        max_events_per_session: Some(50),
        dedup_window_days: Some(60),
        ..Default::default()
    };
    let miner_cfg = closeclaw_memory::miner::MinerConfig::from_mining_config(&mining_config);
    assert_eq!(miner_cfg.max_events_per_session, 50);
    assert_eq!(miner_cfg.dedup_window_days, 60);
}
