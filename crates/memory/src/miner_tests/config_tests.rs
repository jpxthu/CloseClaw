use crate::miner::{MemoryMiner, MinerConfig};
use crate::miner_llm::MockMinerLlmCaller;
use closeclaw_config::agents::MiningConfig;

fn make_miner(config: MinerConfig) -> MemoryMiner {
    let tmp = tempfile::TempDir::new().unwrap();
    let llm = Box::new(MockMinerLlmCaller::default());
    crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md")
}

// ── MinerConfig from_mining_config tests ───────────────────────────────

#[test]
fn test_miner_config_from_mining_config() {
    let mc = MiningConfig {
        enabled: Some(true),
        max_events_per_session: Some(15),
        dedup_window_days: Some(60),
        transcript_clean_rules: closeclaw_config::agents::TranscriptCleanRules {
            min_turns: Some(3),
            min_owner_msgs: Some(4),
            format: Some("plain".to_string()),
        },
        ..Default::default()
    };
    let config = MinerConfig::from_mining_config(&mc);
    assert!(config.enabled);
    assert_eq!(config.max_events_per_session, 15);
    assert_eq!(config.dedup_window_days, 60);
    assert_eq!(config.clean_rules.min_turns, Some(3));
}

#[test]
fn test_miner_config_defaults() {
    let mc = MiningConfig::default();
    let config = MinerConfig::from_mining_config(&mc);
    assert!(!config.enabled);
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
}

#[test]
fn test_miner_config_from_mining_config_copies_model() {
    let mc = MiningConfig {
        model: Some("gpt-4o-mini".to_string()),
        ..Default::default()
    };
    let config = MinerConfig::from_mining_config(&mc);
    assert_eq!(config.model.as_deref(), Some("gpt-4o-mini"));
}

#[test]
fn test_miner_config_from_mining_config_none_model() {
    let mc = MiningConfig::default();
    let config = MinerConfig::from_mining_config(&mc);
    assert_eq!(config.model, None);
}

#[test]
fn test_miner_config_from_mining_config_empty_string_model() {
    let mc = MiningConfig {
        model: Some(String::new()),
        ..Default::default()
    };
    let config = MinerConfig::from_mining_config(&mc);
    assert_eq!(config.model.as_deref(), Some(""));
}

#[test]
fn test_miner_config_default_model_is_none() {
    let config = MinerConfig::default();
    assert_eq!(config.model, None);
}

// ── MinerConfig default value tests ────────────────────────────────────

#[test]
fn test_miner_config_from_mining_config_none_values() {
    let mc = MiningConfig::default();
    let config = MinerConfig::from_mining_config(&mc);
    assert!(!config.enabled);
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
}

#[test]
fn test_miner_config_default_values() {
    let config = MinerConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
}

// ── Config hot-reload tests ────────────────────────────────────────────

#[test]
fn test_update_config_reflects_new_enabled() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = MinerConfig {
        enabled: false,
        ..Default::default()
    };
    let llm = Box::new(MockMinerLlmCaller::default());
    let miner = crate::miner::MemoryMiner::new(config, llm, tmp.path().join("db"), "memory.md");
    assert!(!miner.is_enabled(), "should start disabled");

    let new_config = MinerConfig {
        enabled: true,
        ..Default::default()
    };
    miner.update_config(new_config);
    assert!(miner.is_enabled(), "should be enabled after update_config");
}

// ── MemoryMiner model getter tests ─────────────────────────────────────

#[test]
fn test_model_returns_none_when_unconfigured() {
    let miner = make_miner(MinerConfig::default());
    assert_eq!(miner.model(), None, "model should be None by default");
}

#[test]
fn test_model_returns_configured_value() {
    let config = MinerConfig {
        model: Some("gpt-4o".to_string()),
        ..Default::default()
    };
    let miner = make_miner(config);
    assert_eq!(miner.model().as_deref(), Some("gpt-4o"));
}

#[test]
fn test_model_returns_empty_string() {
    let config = MinerConfig {
        model: Some(String::new()),
        ..Default::default()
    };
    let miner = make_miner(config);
    assert_eq!(miner.model().as_deref(), Some(""));
}

#[test]
fn test_update_config_propagates_model() {
    let miner = make_miner(MinerConfig::default());
    assert_eq!(miner.model(), None);

    let new_config = MinerConfig {
        model: Some("claude-3.5-sonnet".to_string()),
        ..Default::default()
    };
    miner.update_config(new_config);
    assert_eq!(miner.model().as_deref(), Some("claude-3.5-sonnet"));
}

#[test]
fn test_update_config_clears_model() {
    let config = MinerConfig {
        model: Some("gpt-4o".to_string()),
        ..Default::default()
    };
    let miner = make_miner(config);
    assert_eq!(miner.model().as_deref(), Some("gpt-4o"));

    let new_config = MinerConfig {
        model: None,
        ..Default::default()
    };
    miner.update_config(new_config);
    assert_eq!(miner.model(), None);
}

#[test]
fn test_per_agent_override_model() {
    let miner_a = make_miner(MinerConfig {
        model: Some("model-a".to_string()),
        ..Default::default()
    });
    let miner_b = make_miner(MinerConfig {
        model: Some("model-b".to_string()),
        ..Default::default()
    });
    assert_eq!(miner_a.model().as_deref(), Some("model-a"));
    assert_eq!(miner_b.model().as_deref(), Some("model-b"));
}
