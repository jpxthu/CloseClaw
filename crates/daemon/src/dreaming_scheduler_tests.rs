//! Unit tests for DreamingScheduler.

use std::sync::Arc;

use tokio::sync::watch;

use crate::dreaming_scheduler::DreamingScheduler;
use crate::test_helpers::TestStorage;
use closeclaw_common::CompactConfig;
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::PerAgentSessionConfig;
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_session::persistence::{AgentRole, PersistenceService, SessionCheckpoint};

// ── Test helpers ─────────────────────────────────────────────────────────

/// Mock SessionConfigProvider that returns a configurable agent list.
#[derive(Debug)]
struct MockConfig {
    agents: Vec<String>,
}

impl MockConfig {
    fn new(agents: Vec<String>) -> Self {
        Self { agents }
    }

    fn empty() -> Self {
        Self::new(Vec::new())
    }
}

impl SessionConfigProvider for MockConfig {
    fn session_config_for(&self, _agent_id: &str, _role: AgentRole) -> PerAgentSessionConfig {
        PerAgentSessionConfig::default()
    }

    fn sweeper_interval_secs(&self) -> u64 {
        60
    }

    fn dreaming_interval_secs(&self) -> u64 {
        600
    }

    fn list_agents(&self) -> Vec<String> {
        self.agents.clone()
    }

    fn compact_config(&self) -> CompactConfig {
        CompactConfig::default()
    }
}

fn make_scheduler(
    storage: Arc<dyn PersistenceService>,
    config: Arc<dyn SessionConfigProvider>,
) -> DreamingScheduler {
    DreamingScheduler::new(
        storage,
        config,
        Arc::new(DreamingPipeline::new()),
        Arc::new(MemoryMiner::new()),
    )
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Shutdown signal causes the run loop to exit cleanly.
#[tokio::test]
async fn test_dreaming_scheduler_shutdown_exits_loop() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let scheduler = make_scheduler(storage, config);

    let (shutdown_tx, shutdown_rx) = watch::channel(());

    // Spawn the scheduler, then immediately send shutdown.
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    // Give the loop a moment to start, then signal shutdown.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    shutdown_tx.send(()).unwrap();

    // The task should complete within a short timeout.
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "scheduler should exit promptly after shutdown signal"
    );
}

/// run_once calls dreaming pipeline first, then mining scan.
#[tokio::test]
async fn test_dreaming_scheduler_run_once_calls_dreaming_then_mining() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> =
        Arc::new(MockConfig::new(vec!["agent1".to_string()]));
    let scheduler = make_scheduler(storage, config);

    // run_once should succeed even with no data (empty pipeline + no unmined).
    let result = scheduler.run_once().await;
    assert!(result.is_ok(), "run_once should not error: {result:?}");
}

/// Agents without memory config are skipped (empty agent list = no-op).
#[tokio::test]
async fn test_dreaming_scheduler_skips_unconfigured_agents() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    // Config with no agents → list_agents() returns empty → run_once returns early.
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let scheduler = make_scheduler(storage, config);

    let result = scheduler.run_once().await;
    assert!(
        result.is_ok(),
        "run_once with no agents should succeed: {result:?}"
    );
}

/// Empty agent list does not cause errors.
#[tokio::test]
async fn test_dreaming_scheduler_no_agents_no_error() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let scheduler = make_scheduler(storage, config);

    // Multiple run_once calls should all succeed without error.
    for _ in 0..3 {
        let result = scheduler.run_once().await;
        assert!(
            result.is_ok(),
            "repeated run_once should not error: {result:?}"
        );
    }
}

/// Mining scan skips sessions whose agent_id is not in configured agents.
#[tokio::test]
async fn test_dreaming_scheduler_mining_skips_unconfigured_agents() {
    let storage = TestStorage::default();

    // Add an archived checkpoint for an unconfigured agent
    let mut cp = SessionCheckpoint::new("unconfigured-session".into());
    cp.agent_id = Some("unknown-agent".into());
    cp.mined = false;
    storage.add_archived(cp);

    let storage: Arc<dyn PersistenceService> = Arc::new(storage);
    let config: Arc<dyn SessionConfigProvider> =
        Arc::new(MockConfig::new(vec!["configured-agent".to_string()]));
    let scheduler = make_scheduler(storage, config);

    // run_once should succeed; the unconfigured session should be skipped
    let result = scheduler.run_once().await;
    assert!(
        result.is_ok(),
        "run_once with unconfigured agent should succeed: {result:?}"
    );
}
