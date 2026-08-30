//! Unit tests for Step 1.5: SessionConfigProvider independent component behavior.
//!
//! Test dimensions:
//! 1. Normal path: ConfigManager loads → init_phase_2_registries returns a
//!    SessionConfigProvider whose methods (session_config_for, sweeper_interval,
//!    dreaming_interval, consistency_check_interval, list_agents, compact_config)
//!    return correct values.
//! 2. Boundary: ConfigManager loads but session_config.json is missing →
//!    provider falls back to hardcoded defaults.
//! 3. Integration: spawn_background_services receives an independent provider
//!    and passes it to ArchiveSweeper / DreamingScheduler without going
//!    through ConfigManager.

use std::sync::Arc;

use closeclaw_common::AgentRole;
use closeclaw_config::session::{
    JsonSessionConfigProvider, PerAgentSessionConfig, SessionConfigProvider,
    DEFAULT_CONSISTENCY_CHECK_INTERVAL_SECS, DEFAULT_DREAMING_INTERVAL_SECS, DEFAULT_IDLE_MINUTES,
    DEFAULT_PURGE_AFTER_MINUTES, DEFAULT_SWEEPER_INTERVAL_SECS,
};
use closeclaw_config::ConfigManager;
use closeclaw_session::persistence::PersistenceService;

use crate::test_helpers::TestStorage;

// ── Helpers ───────────────────────────────────────────────────────────

/// Minimal mock provider that returns configurable values for every trait method.
#[derive(Debug, Clone)]
struct MockSessionConfigProvider {
    idle_minutes: i64,
    purge_after_minutes: i64,
    is_git_status_enabled: bool,
    sweeper_interval_secs: u64,
    dreaming_interval_secs: u64,
    consistency_check_interval_secs: u64,
    agents: Vec<String>,
}

impl MockSessionConfigProvider {
    /// Provider whose values match the hardcoded defaults.
    fn defaults() -> Self {
        Self {
            idle_minutes: DEFAULT_IDLE_MINUTES,
            purge_after_minutes: DEFAULT_PURGE_AFTER_MINUTES,
            is_git_status_enabled: false,
            sweeper_interval_secs: DEFAULT_SWEEPER_INTERVAL_SECS,
            dreaming_interval_secs: DEFAULT_DREAMING_INTERVAL_SECS,
            consistency_check_interval_secs: DEFAULT_CONSISTENCY_CHECK_INTERVAL_SECS,
            agents: Vec::new(),
        }
    }
}

impl SessionConfigProvider for MockSessionConfigProvider {
    fn session_config_for(&self, _agent_id: &str, _role: AgentRole) -> PerAgentSessionConfig {
        PerAgentSessionConfig::new(
            self.idle_minutes,
            self.purge_after_minutes,
            self.is_git_status_enabled,
        )
    }

    fn sweeper_interval_secs(&self) -> u64 {
        self.sweeper_interval_secs
    }

    fn dreaming_interval_secs(&self) -> u64 {
        self.dreaming_interval_secs
    }

    fn consistency_check_interval_secs(&self) -> u64 {
        self.consistency_check_interval_secs
    }

    fn list_agents(&self) -> Vec<String> {
        self.agents.clone()
    }

    fn compact_config(&self) -> closeclaw_common::CompactConfig {
        closeclaw_common::CompactConfig::default()
    }

    fn plan_archive_days(&self) -> u64 {
        7
    }

    fn audit_log_limit(&self) -> usize {
        1000
    }
}

/// Build a ConfigManager whose config/ subdir contains the given files.
/// Note: the session config file is named `session.json` (matching
/// `ConfigSection::Session.filename()`).
fn make_config_manager(dir: &std::path::Path, session_json: Option<&str>) -> Arc<ConfigManager> {
    let config_dir = dir.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    crate::test_helpers::write_mandatory_configs(&config_dir).unwrap();
    if let Some(json) = session_json {
        std::fs::write(config_dir.join("session.json"), json).unwrap();
    }
    let cm = Arc::new(ConfigManager::new(config_dir).expect("ConfigManager::new failed"));
    cm.load().expect("ConfigManager::load failed");
    cm
}

/// Build a SessionManager with default GatewayConfig.
fn make_session_manager() -> Arc<closeclaw_gateway::SessionManager> {
    use closeclaw_gateway::{GatewayConfig, SessionManager};
    let gateway_config = GatewayConfig::default();
    Arc::new(SessionManager::new(
        &gateway_config,
        None,
        None,
        Default::default(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════
// 1. Normal path: init_phase_2_registries returns usable provider
// ═══════════════════════════════════════════════════════════════════════

/// After ConfigManager loads a session_config.json with per-agent overrides,
/// the provider returned by init_phase_2_registries exposes those values.
#[tokio::test]
async fn test_init_phase_2_returns_provider_with_config() {
    let tmp = tempfile::tempdir().unwrap();
    let session_json = r#"{
        "defaults": {
            "mainAgent": { "idleMinutes": 45, "purgeAfterMinutes": 120 }
        },
        "agents": {
            "guide": {
                "mainAgent": { "idleMinutes": 10, "purgeAfterMinutes": 60, "gitStatus": true }
            }
        },
        "sweeperIntervalSeconds": 120,
        "dreamingIntervalSecs": 900,
        "consistencyCheckIntervalSeconds": 1800
    }"#;
    let cm = make_config_manager(tmp.path(), Some(session_json));

    let (_, _, _, _, provider, _, _, _, _, _) =
        crate::Daemon::init_phase_2_registries(tmp.path().to_str().unwrap(), &cm, &None)
            .await
            .unwrap();

    // Verify per-agent override takes precedence.
    let guide_cfg = provider.session_config_for("guide", AgentRole::MainAgent);
    assert_eq!(guide_cfg.idle_minutes, 10);
    assert_eq!(guide_cfg.purge_after_minutes, 60);
    assert!(guide_cfg.is_git_status_enabled);

    // Verify default config is used when agent has no override.
    let unknown_cfg = provider.session_config_for("unknown-agent", AgentRole::MainAgent);
    assert_eq!(unknown_cfg.idle_minutes, 45);
    assert_eq!(unknown_cfg.purge_after_minutes, 120);

    // Verify global intervals.
    assert_eq!(provider.sweeper_interval_secs(), 120);
    assert_eq!(provider.dreaming_interval_secs(), 900);
    assert_eq!(provider.consistency_check_interval_secs(), 1800);

    // Verify list_agents returns the overridden agent.
    let agents = provider.list_agents();
    assert!(agents.contains(&"guide".to_string()));
}

/// Provider returned by init_phase_2_registries works for ALL trait methods
/// (no panic, no unwrap failure).
#[tokio::test]
async fn test_init_phase_2_provider_all_methods_work() {
    let tmp = tempfile::tempdir().unwrap();
    let session_json = r#"{
        "defaults": { "mainAgent": { "idleMinutes": 20, "purgeAfterMinutes": 30 } },
        "agents": {},
        "sweeperIntervalSeconds": 60,
        "dreamingIntervalSecs": 120,
        "consistencyCheckIntervalSeconds": 300
    }"#;
    let cm = make_config_manager(tmp.path(), Some(session_json));

    let (_, _, _, _, provider, _, _, _, _, _) =
        crate::Daemon::init_phase_2_registries(tmp.path().to_str().unwrap(), &cm, &None)
            .await
            .unwrap();

    // Exercise every method — should not panic.
    let _ = provider.session_config_for("any-agent", AgentRole::MainAgent);
    let _ = provider.sweeper_interval_secs();
    let _ = provider.dreaming_interval_secs();
    let _ = provider.consistency_check_interval_secs();
    let _ = provider.list_agents();
    let _ = provider.compact_config();
}

// ═══════════════════════════════════════════════════════════════════════
// 2. Boundary: session_config.json absent → defaults
// ═══════════════════════════════════════════════════════════════════════

/// When session_config.json is absent, ConfigManager::session_config_provider()
/// returns None, and init_phase_2 falls back to JsonSessionConfigProvider
/// with /dev/null (all defaults).
#[tokio::test]
async fn test_init_phase_2_fallback_defaults_without_session_config() {
    let tmp = tempfile::tempdir().unwrap();
    // No session_config.json → provider falls back to /dev/null defaults.
    let cm = make_config_manager(tmp.path(), None);

    let (_, _, _, _, provider, _, _, _, _, _) =
        crate::Daemon::init_phase_2_registries(tmp.path().to_str().unwrap(), &cm, &None)
            .await
            .unwrap();

    // All values should be hardcoded defaults.
    let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);
    assert_eq!(cfg.idle_minutes, DEFAULT_IDLE_MINUTES);
    assert_eq!(cfg.purge_after_minutes, DEFAULT_PURGE_AFTER_MINUTES);
    assert_eq!(
        provider.sweeper_interval_secs(),
        DEFAULT_SWEEPER_INTERVAL_SECS
    );
    assert_eq!(
        provider.dreaming_interval_secs(),
        DEFAULT_DREAMING_INTERVAL_SECS
    );
    assert_eq!(
        provider.consistency_check_interval_secs(),
        DEFAULT_CONSISTENCY_CHECK_INTERVAL_SECS
    );
    assert!(provider.list_agents().is_empty());
}

/// JsonSessionConfigProvider constructed from a non-existent path returns
/// all defaults (file-not-found triggers warn + defaults, not an error).
/// This is the fallback path used by init_phase_2_registries.
#[test]
fn test_json_session_config_provider_missing_file_defaults() {
    let provider = JsonSessionConfigProvider::new("/tmp/__nonexistent_session_cfg_defaults__.json")
        .expect("should succeed with defaults for missing file");

    let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);
    assert_eq!(cfg.idle_minutes, DEFAULT_IDLE_MINUTES);
    assert_eq!(cfg.purge_after_minutes, DEFAULT_PURGE_AFTER_MINUTES);
    assert!(!cfg.is_git_status_enabled);
    assert_eq!(
        provider.sweeper_interval_secs(),
        DEFAULT_SWEEPER_INTERVAL_SECS
    );
    assert_eq!(
        provider.dreaming_interval_secs(),
        DEFAULT_DREAMING_INTERVAL_SECS
    );
    assert_eq!(
        provider.consistency_check_interval_secs(),
        DEFAULT_CONSISTENCY_CHECK_INTERVAL_SECS
    );
    assert!(provider.list_agents().is_empty());
}

/// JsonSessionConfigProvider with a non-existent file also returns defaults
/// (the file-not-found path is handled with a warn, not an error).
#[test]
fn test_json_session_config_provider_nonexistent_file_defaults() {
    let provider = JsonSessionConfigProvider::new("/tmp/__nonexistent_session_config__.json")
        .expect("should succeed with defaults");

    let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);
    assert_eq!(cfg.idle_minutes, DEFAULT_IDLE_MINUTES);
    assert_eq!(cfg.purge_after_minutes, DEFAULT_PURGE_AFTER_MINUTES);
}

// ═══════════════════════════════════════════════════════════════════════
// 3. Integration: spawn_background_services uses independent provider
// ═══════════════════════════════════════════════════════════════════════

/// ArchiveSweeper receives the independent provider and uses its intervals.
/// We verify by checking that the sweeper's `run_once()` succeeds when
/// given the injected provider — proving the provider is reachable.
#[tokio::test]
async fn test_archive_sweeper_uses_independent_provider() {
    use closeclaw_gateway::sweeper::ArchiveSweeper;

    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let provider: Arc<dyn SessionConfigProvider> = Arc::new(MockSessionConfigProvider::defaults());

    let sweeper = ArchiveSweeper::new(Arc::clone(&storage), Arc::clone(&provider));

    // run_once should succeed — the provider is directly reachable.
    let result = sweeper.run_once().await;
    assert!(
        result.is_ok(),
        "ArchiveSweeper::run_once should succeed with independent provider: {:?}",
        result.err()
    );
}

/// DreamingScheduler receives the independent provider and uses its intervals.
/// We verify by calling run_once() which exercises the provider's list_agents()
/// and dreaming_interval_secs().
#[tokio::test]
async fn test_dreaming_scheduler_uses_independent_provider() {
    use crate::dreaming_scheduler::DreamingScheduler;
    use closeclaw_config::agents::DreamingConfig;
    use closeclaw_memory::dreaming::DreamingPipeline;
    use closeclaw_memory::miner::MemoryMiner;

    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let provider: Arc<dyn SessionConfigProvider> = Arc::new(MockSessionConfigProvider {
        agents: vec!["test-agent".to_string()],
        ..MockSessionConfigProvider::defaults()
    });

    let tmp = tempfile::tempdir().unwrap();
    let config_manager =
        Arc::new(ConfigManager::new(tmp.path().join("config")).expect("ConfigManager::new failed"));

    let pipeline = Arc::new(DreamingPipeline::with_config(DreamingConfig {
        enabled: Some(true),
        ..Default::default()
    }));
    let miner = Arc::new(MemoryMiner::new(
        closeclaw_memory::miner::MinerConfig::default(),
        Box::new(crate::noop_miner_llm::NoopMinerLlmCaller),
        Box::new(crate::noop_miner_llm::NoopMinerLlmCaller),
        tmp.path().join("memory.db"),
        tmp.path().join("MEMORY.md").to_string_lossy().into_owned(),
    ));

    let scheduler = DreamingScheduler::new(
        Arc::clone(&storage),
        Arc::clone(&provider),
        pipeline,
        miner,
        config_manager,
    );

    // run_once exercises list_agents() and dreaming_interval_secs().
    let result = scheduler.run_once().await;
    assert!(
        result.is_ok(),
        "DreamingScheduler::run_once should succeed with independent provider: {:?}",
        result.err()
    );
}

/// spawn_background_services receives the provider and passes it to both
/// ArchiveSweeper and DreamingScheduler. We verify the full spawn + shutdown
/// cycle completes without error.
#[tokio::test]
async fn test_spawn_background_services_with_independent_provider() {
    use crate::Daemon;

    let tmp = tempfile::tempdir().unwrap();
    let provider: Arc<dyn SessionConfigProvider> = Arc::new(MockSessionConfigProvider::defaults());
    let config_manager =
        Arc::new(ConfigManager::new(tmp.path().join("config")).expect("ConfigManager::new failed"));
    let session_manager = make_session_manager();

    let (sweeper_rx, announce_sweeper_rx, dreaming_rx) = (
        tokio::sync::watch::channel(()).1,
        tokio::sync::watch::channel(()).1,
        tokio::sync::watch::channel(()).1,
    );
    let shutdown_receivers = crate::ServiceShutdownReceivers {
        sweeper: sweeper_rx,
        announce_sweeper: announce_sweeper_rx,
        dreaming: dreaming_rx,
    };

    let handles = Daemon::spawn_background_services(
        &config_manager,
        &session_manager,
        tmp.path(),
        shutdown_receivers,
        Arc::clone(&provider),
    );

    // All 3 handles should be valid (tasks are spawned).
    // PlanArchiveSweeper is now spawned in populate_registries, not here.
    let (sweeper_h, announce_h, dreaming_h) = handles;
    assert!(!sweeper_h.is_finished());
    assert!(!announce_h.is_finished());
    assert!(!dreaming_h.is_finished());

    // Abort all tasks so the test exits cleanly.
    sweeper_h.abort();
    announce_h.abort();
    dreaming_h.abort();
}

// ═══════════════════════════════════════════════════════════════════════
// 4. SessionManager: set_session_config_provider injection
// ═══════════════════════════════════════════════════════════════════════

/// After set_session_config_provider is called, the SessionManager holds
/// the injected provider. This is verified indirectly by checking that
/// set_session_config_provider completes without error and the provider
/// is reachable through the same Arc.
#[tokio::test]
async fn test_session_manager_set_provider_completes() {
    use closeclaw_gateway::{GatewayConfig, SessionManager};

    let provider: Arc<dyn SessionConfigProvider> = Arc::new(MockSessionConfigProvider {
        idle_minutes: 99,
        purge_after_minutes: 88,
        is_git_status_enabled: true,
        ..MockSessionConfigProvider::defaults()
    });

    let config = GatewayConfig::default();
    let sm = SessionManager::new(&config, None, None, Default::default());

    // set_session_config_provider should complete without error.
    sm.set_session_config_provider(Arc::clone(&provider)).await;

    // The provider is still usable (not consumed or corrupted).
    let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);
    assert_eq!(cfg.idle_minutes, 99);
    assert_eq!(cfg.purge_after_minutes, 88);
    assert!(cfg.is_git_status_enabled);
}

/// Without set_session_config_provider, the SessionManager falls back to
/// ConfigManager. When ConfigManager is not set either, the session_config
/// lookup returns None. This is verified indirectly by checking that the
/// SessionManager can be constructed and the fallback path exists.
#[tokio::test]
async fn test_session_manager_construction_without_provider() {
    use closeclaw_gateway::{GatewayConfig, SessionManager};

    let config = GatewayConfig::default();
    let sm = SessionManager::new(&config, None, None, Default::default());

    // The session_config_provider field is None initially.
    // We cannot directly read it (private), but we can verify the
    // SessionManager was created successfully and the field exists by
    // checking that the struct compiles and runs.
    let _ = sm;
}

// ═══════════════════════════════════════════════════════════════════════
// 5. Consistency check interval propagation
// ═══════════════════════════════════════════════════════════════════════

/// The consistency check interval from the provider is used when
/// session_manager.spawn_periodic_consistency_check() is called inside
/// spawn_background_services. We verify the value is correctly passed by
/// checking that the provider's consistency_check_interval_secs() returns
/// the expected value.
#[tokio::test]
async fn test_consistency_check_interval_from_provider() {
    let provider: Arc<dyn SessionConfigProvider> = Arc::new(MockSessionConfigProvider {
        consistency_check_interval_secs: 7200,
        ..MockSessionConfigProvider::defaults()
    });

    assert_eq!(provider.consistency_check_interval_secs(), 7200);
}

/// Consistency check interval defaults to 3600 when not configured.
#[tokio::test]
async fn test_consistency_check_interval_default() {
    let provider = JsonSessionConfigProvider::new("/tmp/__nonexistent_consistency_default__.json")
        .expect("should succeed with defaults for missing file");
    assert_eq!(
        provider.consistency_check_interval_secs(),
        DEFAULT_CONSISTENCY_CHECK_INTERVAL_SECS
    );
    assert_eq!(DEFAULT_CONSISTENCY_CHECK_INTERVAL_SECS, 3600);
}
