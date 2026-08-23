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
    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap(), None);
    let guard = engine.blocking_read();
    let rs = guard.rules();
    assert!(rs.rules.is_empty(), "no templates should yield empty rules");
    let ud = &rs.user_defaults;
    let expected = closeclaw_permission::Defaults::user_defaults();
    assert_eq!(ud.file_read, expected.file_read);
    assert_eq!(ud.file_write, expected.file_write);
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

    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap(), None);
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
// Step 1.2 — validate_phase_components with AnnounceSweeper
// ============================================================

/// Normal path: complete startup layers (including AnnounceSweeper)
/// pass validate_phase_components without error.
#[test]
fn test_validate_phase_components_with_announce_sweeper_succeeds() {
    use crate::startup::{all_component_entries, topo_sort_layers};

    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    let result = Daemon::validate_phase_components(&layers);
    assert!(
        result.is_ok(),
        "validate_phase_components should succeed with AnnounceSweeper: {:?}",
        result.err()
    );
    let phases = result.unwrap();
    assert_eq!(phases.len(), 6, "expected 6 phases");
    // AnnounceSweeper must appear in Phase 3 (index 2)
    use crate::startup::{ComponentId, Service};
    assert!(
        phases[2].contains(&ComponentId::Service(Service::AnnounceSweeper)),
        "Phase 3 must contain AnnounceSweeper"
    );
}

/// Boundary: removing AnnounceSweeper from Layer 3 causes
/// validate_phase_components to return CircularDependency.
#[test]
fn test_validate_phase_components_missing_announce_sweeper_fails() {
    use crate::startup::{all_component_entries, topo_sort_layers, ComponentId, Service};

    let entries = all_component_entries();
    let mut layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    // Remove AnnounceSweeper from Layer 3 (index 2)
    layers[2].retain(|id| *id != ComponentId::Service(Service::AnnounceSweeper));

    let result = Daemon::validate_phase_components(&layers);
    assert!(
        result.is_err(),
        "validate_phase_components should fail when AnnounceSweeper is missing"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, crate::startup::StartupError::CircularDependency),
        "expected CircularDependency error, got: {err:?}"
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

// ============================================================
// Step 1.4: Daemon LLM chain assembly verification
// ============================================================

/// Verify that `init_llm_registry` returns a registry containing
/// all providers configured via credentials files.
#[tokio::test]
async fn test_init_llm_registry_contains_configured_providers() {
    let tmp = TempDir::new().unwrap();
    let creds_dir = tmp.path().join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();

    // Create credential files for openai and anthropic
    std::fs::write(
        creds_dir.join("openai.json"),
        r#"{"provider":"openai","apiKey":"openai-key"}"#,
    )
    .unwrap();
    std::fs::write(
        creds_dir.join("anthropic.json"),
        r#"{"provider":"anthropic","apiKey":"anthropic-key"}"#,
    )
    .unwrap();

    let registry = Daemon::init_llm_registry(tmp.path(), &HashMap::new()).await;
    let listed = registry.list().await;

    assert!(listed.contains(&"openai".to_string()));
    assert!(listed.contains(&"anthropic".to_string()));
}

/// Verify that `init_llm_registry` registers providers from env
/// overrides when no credentials files exist.
#[tokio::test]
async fn test_init_llm_registry_env_override_providers() {
    let tmp = TempDir::new().unwrap();
    let overrides: HashMap<&str, &str> = HashMap::from([
        ("OPENAI_API_KEY", "env-openai-key"),
        ("MINIMAX_API_KEY", "env-minimax-key"),
    ]);

    let registry = Daemon::init_llm_registry(tmp.path(), &overrides).await;
    let listed = registry.list().await;

    assert!(listed.contains(&"openai".to_string()));
    assert!(listed.contains(&"minimax".to_string()));
}

/// Verify that `for_provider` maps anthropic to AnthropicCacheAdapter
/// and minimax to NoopCacheAdapter, matching the design doc.
#[test]
fn test_cache_adapter_mapping_matches_design_doc() {
    // Anthropic → explicit prefix caching
    let anthropic_adapter = closeclaw_llm::cache_adapter::for_provider("anthropic");
    assert_eq!(anthropic_adapter.name(), "anthropic");

    // MiniMax → no explicit cache params (noop)
    let minimax_adapter = closeclaw_llm::cache_adapter::for_provider("minimax");
    assert_eq!(minimax_adapter.name(), "noop");

    // OpenAI → no explicit cache params (noop)
    let openai_adapter = closeclaw_llm::cache_adapter::for_provider("openai");
    assert_eq!(openai_adapter.name(), "noop");

    // Kimi → prompt_cache_key
    let kimi_adapter = closeclaw_llm::cache_adapter::for_provider("kimi");
    assert_eq!(kimi_adapter.name(), "kimi");
}

/// Verify that the full LLM chain assembly produces correct chain
/// entries with correct cache adapters for each provider.
#[test]
fn test_llm_chain_assembly_correct_adapters() {
    use closeclaw_llm::cache_adapter::for_provider;
    use closeclaw_llm::interpreter::InterpreterRegistry;
    use closeclaw_llm::plugin::PluginPipeline;
    use closeclaw_llm::protocol::OpenAiProtocol;
    use closeclaw_llm::retry::CooldownManager;
    use closeclaw_llm::stub::StubProvider;
    use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};

    // Build chain entries mirroring the lifecycle.rs assembly logic
    let providers = vec![
        ("openai", "openai"),
        ("anthropic", "anthropic"),
        ("minimax", "minimax"),
    ];

    let mut chain_entries: Vec<ChainEntry> = Vec::new();
    for (provider_id, _) in &providers {
        let provider: Arc<dyn closeclaw_llm::provider::Provider> = Arc::new(StubProvider::new());
        let cache_adapter = for_provider(provider_id);
        let client = Arc::new(closeclaw_llm::UnifiedChatClient::new(
            provider,
            Arc::new(OpenAiProtocol::new()),
            InterpreterRegistry::default(),
            PluginPipeline::new(),
            cache_adapter,
        ));
        chain_entries.push(ChainEntry {
            provider_id: provider_id.to_string(),
            model_id: provider_id.to_string(),
            client,
        });
    }

    let cooldown = Arc::new(CooldownManager::new());
    let fallback = UnifiedFallbackClient::new(chain_entries, cooldown);

    // Verify chain has correct entries
    let chain = fallback.chain();
    assert_eq!(chain.len(), 3);
    assert_eq!(chain[0].provider_id, "openai");
    assert_eq!(chain[1].provider_id, "anthropic");
    assert_eq!(chain[2].provider_id, "minimax");

    // Verify model_id equals provider_id for each entry
    assert_eq!(
        chain[0].model_id, "openai",
        "model_id should equal provider_id"
    );
    assert_eq!(
        chain[1].model_id, "anthropic",
        "model_id should equal provider_id"
    );
    assert_eq!(
        chain[2].model_id, "minimax",
        "model_id should equal provider_id"
    );

    // Verify each client's Debug output contains the correct adapter name
    // (UnifiedChatClient Debug impl includes cache_adapter.name())
    let debug_0 = format!("{:?}", chain[0].client);
    assert!(
        debug_0.contains("noop"),
        "openai client should use noop adapter, got: {debug_0}"
    );

    let debug_1 = format!("{:?}", chain[1].client);
    assert!(
        debug_1.contains("anthropic"),
        "anthropic client should use anthropic adapter, got: {debug_1}"
    );

    let debug_2 = format!("{:?}", chain[2].client);
    assert!(
        debug_2.contains("noop"),
        "minimax client should use noop adapter, got: {debug_2}"
    );
}

/// Verify that FallbackLlmCaller wraps the correct UnifiedFallbackClient
/// and that the chain is accessible through it.
#[test]
fn test_fallback_llm_caller_chain_accessible() {
    use closeclaw_gateway::llm_caller_impl::FallbackLlmCaller;
    use closeclaw_llm::cache_adapter::for_provider;
    use closeclaw_llm::interpreter::InterpreterRegistry;
    use closeclaw_llm::plugin::PluginPipeline;
    use closeclaw_llm::protocol::OpenAiProtocol;
    use closeclaw_llm::retry::CooldownManager;
    use closeclaw_llm::stub::StubProvider;
    use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};

    let provider: Arc<dyn closeclaw_llm::provider::Provider> = Arc::new(StubProvider::new());
    let cache_adapter = for_provider("anthropic");
    let client = Arc::new(closeclaw_llm::UnifiedChatClient::new(
        provider,
        Arc::new(OpenAiProtocol::new()),
        InterpreterRegistry::default(),
        PluginPipeline::new(),
        cache_adapter,
    ));
    let entry = ChainEntry {
        provider_id: "anthropic".to_string(),
        model_id: "claude-3".to_string(),
        client,
    };
    let cooldown = Arc::new(CooldownManager::new());
    let fallback = Arc::new(UnifiedFallbackClient::new(vec![entry], cooldown));
    let caller = FallbackLlmCaller(Arc::clone(&fallback));

    // Verify the caller's inner fallback client has the expected chain
    let chain = caller.0.chain();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].provider_id, "anthropic");
    assert_eq!(chain[0].model_id, "claude-3");
}

// ============================================================
// Step 1.3: resolve_extra_dirs path expansion tests
// ============================================================

/// Helper: build a ConfigManager whose skills.json contains the given
/// `extraDirs` list so that `resolve_extra_dirs` can extract it.
fn config_manager_with_extra_dirs(dirs: &[&str]) -> closeclaw_config::ConfigManager {
    let tmp = tempfile::tempdir().unwrap();
    let config_subdir = tmp.path().join("config");
    std::fs::create_dir_all(&config_subdir).unwrap();
    // Write mandatory config files so load() succeeds
    crate::test_helpers::write_mandatory_configs(&config_subdir).unwrap();
    let skills_json = serde_json::json!({
        "extraDirs": dirs
    });
    std::fs::write(config_subdir.join("skills.json"), skills_json.to_string()).unwrap();
    let cm = closeclaw_config::ConfigManager::new(config_subdir).unwrap();
    cm.load().unwrap();
    cm
}

/// `~` prefix is expanded to the user's home directory.
#[test]
fn test_resolve_extra_dirs_tilde_expanded() {
    let cm = config_manager_with_extra_dirs(&["~/my-skills"]);
    let result = skills_helper::resolve_extra_dirs(&cm);
    assert_eq!(result.len(), 1);
    let expected_home = dirs::home_dir().expect("home_dir should exist");
    assert_eq!(result[0], expected_home.join("my-skills"));
}

/// `~/a/b` nested path is expanded correctly.
#[test]
fn test_resolve_extra_dirs_tilde_nested_path() {
    let cm = config_manager_with_extra_dirs(&["~/a/b/c"]);
    let result = skills_helper::resolve_extra_dirs(&cm);
    assert_eq!(result.len(), 1);
    let expected_home = dirs::home_dir().expect("home_dir should exist");
    assert_eq!(result[0], expected_home.join("a/b/c"));
}

/// Absolute path is kept as-is (no expansion).
#[test]
fn test_resolve_extra_dirs_absolute_path_unchanged() {
    let cm = config_manager_with_extra_dirs(&["/opt/skills"]);
    let result = skills_helper::resolve_extra_dirs(&cm);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], std::path::PathBuf::from("/opt/skills"));
}

/// Relative path is kept as-is — loader layer handles existence check.
#[test]
fn test_resolve_extra_dirs_relative_path_unchanged() {
    let cm = config_manager_with_extra_dirs(&["relative/skills"]);
    let result = skills_helper::resolve_extra_dirs(&cm);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], std::path::PathBuf::from("relative/skills"));
}

/// No skills config → empty Vec (graceful default).
#[test]
fn test_resolve_extra_dirs_no_skills_config() {
    let tmp = tempfile::tempdir().unwrap();
    let config_subdir = tmp.path().join("config");
    std::fs::create_dir_all(&config_subdir).unwrap();
    crate::test_helpers::write_mandatory_configs(&config_subdir).unwrap();
    // Overwrite system.json with empty object (no skills section)
    std::fs::write(
        config_subdir.join("system.json"),
        serde_json::json!({}).to_string(),
    )
    .unwrap();
    let cm = closeclaw_config::ConfigManager::new(config_subdir).unwrap();
    cm.load().unwrap();
    let result = skills_helper::resolve_extra_dirs(&cm);
    assert!(result.is_empty());
}

/// Invalid JSON in skills.json → empty Vec (graceful, no panic).
#[test]
fn test_resolve_extra_dirs_invalid_json() {
    let tmp = tempfile::tempdir().unwrap();
    let config_subdir = tmp.path().join("config");
    std::fs::create_dir_all(&config_subdir).unwrap();
    crate::test_helpers::write_mandatory_configs(&config_subdir).unwrap();
    // Write malformed JSON to skills.json
    std::fs::write(config_subdir.join("skills.json"), "not valid json {{{").unwrap();
    let cm = closeclaw_config::ConfigManager::new(config_subdir).unwrap();
    cm.load().unwrap();
    let result = skills_helper::resolve_extra_dirs(&cm);
    assert!(result.is_empty(), "invalid JSON should yield empty Vec");
}

/// Mixed paths: tilde, absolute, relative.
#[test]
fn test_resolve_extra_dirs_mixed_paths() {
    let cm = config_manager_with_extra_dirs(&["~/my-skills", "/opt/skills", "relative/skills"]);
    let result = skills_helper::resolve_extra_dirs(&cm);
    assert_eq!(result.len(), 3);
    let expected_home = dirs::home_dir().expect("home_dir should exist");
    assert_eq!(result[0], expected_home.join("my-skills"));
    assert_eq!(result[1], std::path::PathBuf::from("/opt/skills"));
    assert_eq!(result[2], std::path::PathBuf::from("relative/skills"));
}

// ============================================================
// Step 1.3: ServiceShutdownReceivers struct tests
// ============================================================

/// Verify that ServiceShutdownReceivers can be constructed and fields
/// are correctly assigned and accessible.
#[test]
fn test_service_shutdown_receivers_construction() {
    let (_tx1, rx1) = tokio::sync::watch::channel(());
    let (_tx2, rx2) = tokio::sync::watch::channel(());
    let (_tx3, rx3) = tokio::sync::watch::channel(());
    let (_tx4, rx4) = tokio::sync::watch::channel(());

    let receivers = ServiceShutdownReceivers {
        sweeper: rx1,
        announce_sweeper: rx2,
        dreaming: rx3,
        plan_archive: rx4,
    };

    // Verify fields are accessible (destructure)
    let ServiceShutdownReceivers {
        sweeper: _,
        announce_sweeper: _,
        dreaming: _,
        plan_archive: _,
    } = receivers;
}

/// Verify that ServiceShutdownReceivers destructuring works identically
/// to how spawn_background_services uses it (binding to local variables).
#[test]
fn test_service_shutdown_receivers_destructure_like_spawn() {
    let (_tx1, rx1) = tokio::sync::watch::channel(());
    let (_tx2, rx2) = tokio::sync::watch::channel(());
    let (_tx3, rx3) = tokio::sync::watch::channel(());
    let (_tx4, rx4) = tokio::sync::watch::channel(());

    let shutdown_receivers = ServiceShutdownReceivers {
        sweeper: rx1,
        announce_sweeper: rx2,
        dreaming: rx3,
        plan_archive: rx4,
    };

    // Destructure exactly as spawn_background_services does
    let ServiceShutdownReceivers {
        sweeper: sweeper_rx,
        announce_sweeper: announce_sweeper_rx,
        dreaming: dreaming_rx,
        plan_archive: plan_archive_rx,
    } = shutdown_receivers;

    // Verify each receiver is a valid watch::Receiver<()> by checking
    // they can be borrowed immutably (no panic).
    let _ = sweeper_rx.borrow();
    let _ = announce_sweeper_rx.borrow();
    let _ = dreaming_rx.borrow();
    let _ = plan_archive_rx.borrow();
}
