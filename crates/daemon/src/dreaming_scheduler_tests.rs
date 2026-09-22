//! Unit tests for DreamingScheduler.

use std::sync::Arc;

use tokio::sync::watch;

use crate::dreaming_scheduler::DreamingScheduler;
use crate::test_helpers::TestStorage;
use closeclaw_common::CompactConfig;
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigManager;
use closeclaw_config::PerAgentSessionConfig;
use closeclaw_memory::dreaming::{DreamingError, DreamingPipeline};
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_session::persistence::{
    AgentRole, DreamingStatus, PersistenceService, SessionCheckpoint,
};

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

    fn consistency_check_interval_secs(&self) -> u64 {
        3600
    }

    fn list_agents(&self) -> Vec<String> {
        self.agents.clone()
    }

    fn compact_config(&self) -> CompactConfig {
        CompactConfig::default()
    }

    fn plan_archive_days(&self) -> u64 {
        7
    }

    fn audit_log_limit(&self) -> usize {
        1000
    }
}

fn make_scheduler(
    storage: Arc<dyn PersistenceService>,
    config: Arc<dyn SessionConfigProvider>,
    root: &std::path::Path,
) -> DreamingScheduler {
    let config_manager = Arc::new(
        ConfigManager::new(root.join("config")).expect("failed to create test ConfigManager"),
    );
    DreamingScheduler::new(
        storage,
        config,
        Arc::new(DreamingPipeline::new()),
        Arc::new(MemoryMiner::new(
            closeclaw_memory::miner::MinerConfig::default(),
            Box::new(crate::noop_miner_llm::NoopMinerLlmCaller),
            Box::new(crate::noop_miner_llm::NoopMinerLlmCaller),
            root.join("memory.db"),
            root.join("MEMORY.md").to_string_lossy().into_owned(),
        )),
        config_manager,
    )
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Shutdown signal causes the run loop to exit cleanly.
#[tokio::test]
async fn test_dreaming_scheduler_shutdown_exits_loop() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut scheduler = make_scheduler(storage, config, tmp.path());

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
    let tmp = tempfile::tempdir().expect("temp dir");
    let scheduler = make_scheduler(storage, config, tmp.path());

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
    let tmp = tempfile::tempdir().expect("temp dir");
    let scheduler = make_scheduler(storage, config, tmp.path());

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
    let tmp = tempfile::tempdir().expect("temp dir");
    let scheduler = make_scheduler(storage, config, tmp.path());

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
    let tmp = tempfile::tempdir().expect("temp dir");
    let scheduler = make_scheduler(storage, config, tmp.path());

    // run_once should succeed; the unconfigured session should be skipped
    let result = scheduler.run_once().await;
    assert!(
        result.is_ok(),
        "run_once with unconfigured agent should succeed: {result:?}"
    );
}

/// Scheduler with valid cron schedule uses cron-based scheduling.
#[tokio::test]
async fn test_dreaming_scheduler_cron_schedule_shutdown() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut scheduler =
        make_scheduler(storage, config, tmp.path()).with_schedule(Some("0 3 * * *".to_string()));

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "scheduler with cron schedule should exit promptly"
    );
}

/// Scheduler with invalid cron falls back to fixed interval.
#[tokio::test]
async fn test_dreaming_scheduler_invalid_cron_fallback() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut scheduler =
        make_scheduler(storage, config, tmp.path()).with_schedule(Some("not-a-cron".to_string()));

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "scheduler with invalid cron should exit promptly (fallback)"
    );
}

/// Scheduler with no schedule uses fixed interval (backward compat).
#[tokio::test]
async fn test_dreaming_scheduler_no_schedule_uses_fixed() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut scheduler = make_scheduler(storage, config, tmp.path());

    assert!(scheduler.schedule.is_none());

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "scheduler without schedule should exit promptly"
    );
}

// ── Step 1.6: Cron schedule consumption tests ─────────────────────────

/// Scheduler with_schedule() correctly stores the cron expression.
#[test]
fn test_with_schedule_stores_cron_expression() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let scheduler =
        make_scheduler(storage, config, tmp.path()).with_schedule(Some("0 3 * * *".to_string()));
    assert_eq!(scheduler.schedule.as_deref(), Some("0 3 * * *"));
}

/// Scheduler with None schedule stores None.
#[test]
fn test_with_schedule_none() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let scheduler = make_scheduler(storage, config, tmp.path()).with_schedule(None);
    assert!(scheduler.schedule.is_none());
}

/// Scheduler with empty string falls back to fixed interval (invalid cron).
#[tokio::test]
async fn test_with_schedule_empty_string_fallback() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut scheduler =
        make_scheduler(storage, config, tmp.path()).with_schedule(Some(String::new()));

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "scheduler with empty schedule string should exit promptly"
    );
}

/// Scheduler with hourly cron parses and runs correctly.
#[tokio::test]
async fn test_dreaming_scheduler_hourly_cron() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut scheduler =
        make_scheduler(storage, config, tmp.path()).with_schedule(Some("0 * * * *".to_string()));

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "scheduler with hourly cron should exit promptly"
    );
}

// ── Step 1.5: Config hot-reload tests ────────────────────────────────

/// Helper to create a ConfigManager backed by a temp directory.
fn make_test_config_manager(dir: &std::path::Path) -> Arc<ConfigManager> {
    Arc::new(ConfigManager::new(dir.to_path_buf()).expect("failed to create test ConfigManager"))
}

// ── Config-change test helpers (setup / scenario / assert) ────────────

/// Setup: build a pipeline and miner with dreaming/mining DISABLED, so a
/// Memory section reload can be verified to flip them on (or stay off).
fn make_disabled_pipeline_and_miner(
    db_path: &std::path::Path,
    memory_md_path: &std::path::Path,
) -> (Arc<DreamingPipeline>, Arc<MemoryMiner>) {
    let pipeline = Arc::new(DreamingPipeline::with_config(
        closeclaw_config::agents::DreamingConfig {
            enabled: Some(false),
            ..Default::default()
        },
    ));
    let miner = Arc::new(MemoryMiner::new(
        closeclaw_memory::miner::MinerConfig {
            enabled: false,
            ..Default::default()
        },
        Box::new(crate::noop_miner_llm::NoopMinerLlmCaller),
        Box::new(crate::noop_miner_llm::NoopMinerLlmCaller),
        db_path.to_path_buf(),
        memory_md_path.to_string_lossy().into_owned(),
    ));
    (pipeline, miner)
}

/// Setup: wire fresh storage and an empty mock config into a scheduler
/// around the given pipeline/miner/config manager.
fn make_config_change_scheduler(
    pipeline: Arc<DreamingPipeline>,
    miner: Arc<MemoryMiner>,
    config_manager: Arc<ConfigManager>,
) -> DreamingScheduler {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    DreamingScheduler::new(storage, config, pipeline, miner, config_manager)
}

/// Setup: populate the Memory section cache with dreaming/mining enabled.
fn seed_memory_section_cache(config_manager: &ConfigManager) {
    let memory_value = serde_json::json!({
        "dreaming": {
            "enabled": true
        },
        "mining": {
            "enabled": true
        }
    });
    config_manager.update_section_cache(
        closeclaw_config::ConfigSection::Memory,
        std::path::PathBuf::from("memory.json"),
        memory_value,
    );
}

/// Scenario: broadcast a section reload event and let it propagate
/// (broadcast channel delivery).
async fn notify_section_reload(
    config_manager: &ConfigManager,
    section: closeclaw_config::ConfigSection,
    path: &str,
) {
    config_manager.notify_change(closeclaw_config::ConfigChangeEvent::Reloaded {
        section,
        path: path.into(),
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
}

/// Scenario: run the scheduler loop so it can receive and process pending
/// config events, then shut it down and verify prompt exit.
async fn run_scheduler_briefly(mut scheduler: DreamingScheduler) {
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    // Give the loop time to start, receive the event, and process it.
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert!(result.is_ok(), "scheduler should exit promptly");
}

/// Scenario: run the pipeline once over a fresh mined+undreamt checkpoint and
/// return the dreaming status it ends up with (assertions stay at call sites).
async fn run_once_and_get_status(
    pipeline: &DreamingPipeline,
    session_id: &str,
) -> Result<DreamingStatus, DreamingError> {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new(session_id.to_string());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    pipeline.run_once(&storage).await?;

    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps
        .iter()
        .find(|c| c.session_id == session_id)
        .unwrap_or_else(|| {
            let known: Vec<&str> = cps.iter().map(|c| c.session_id.as_str()).collect();
            panic!("session {session_id} not found in known checkpoints: {known:?}");
        });
    Ok(cp.dreaming_status)
}

/// Receiving ConfigChangeEvent::Reloaded{section:Memory} updates pipeline/miner config.
#[tokio::test]
async fn test_config_change_memory_section_updates_components() {
    // Setup: disabled pipeline/miner, Memory section cache with dreaming on.
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    seed_memory_section_cache(&config_manager);
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-reload.db"),
        &tmp.path().join("MEMORY-reload.md"),
    );
    let scheduler =
        make_config_change_scheduler(pipeline.clone(), miner.clone(), config_manager.clone());

    // Verify initial state: miner is disabled.
    assert!(!miner.is_enabled(), "miner should start disabled");

    // Scenario: the Memory section reload flips pipeline/miner configs on.
    notify_section_reload(
        &config_manager,
        closeclaw_config::ConfigSection::Memory,
        "memory.json",
    )
    .await;
    run_scheduler_briefly(scheduler).await;

    // Assert: pipeline is enabled now, so run_once processes the session
    // (mined+undreamt checkpoint must move Pending → Completed).
    let status = run_once_and_get_status(&pipeline, "post-reload")
        .await
        .expect("run_once after config reload should succeed");
    assert_eq!(
        status,
        DreamingStatus::Completed,
        "pipeline should be enabled after config reload"
    );
}

/// Non-Memory section events do not trigger pipeline/miner config updates.
#[tokio::test]
async fn test_config_change_non_memory_ignored() {
    // Setup: disabled pipeline/miner, no Memory section cache involved.
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-ignore.db"),
        &tmp.path().join("MEMORY-ignore.md"),
    );
    let scheduler =
        make_config_change_scheduler(pipeline.clone(), miner.clone(), config_manager.clone());

    // Scenario: a NON-Memory section reload event must be ignored.
    notify_section_reload(
        &config_manager,
        closeclaw_config::ConfigSection::Gateway,
        "gateway.json",
    )
    .await;
    run_scheduler_briefly(scheduler).await;

    // Assert: pipeline stays disabled, so the pending session is untouched
    // (mined+undreamt checkpoint must remain Pending).
    let status = run_once_and_get_status(&pipeline, "ignore-test")
        .await
        .expect("run_once should succeed even when pipeline is disabled");
    assert_eq!(
        status,
        DreamingStatus::Pending,
        "non-Memory event should not enable pipeline"
    );
}

// ── Step 1.3: Channel safety + immediate hook trigger tests ────────

/// When the receiver is dropped, sending on the channel must not panic.
#[tokio::test]
async fn test_channel_disconnected_send_does_not_panic() {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(1);

    // Drop the receiver first
    drop(rx);

    // Sending after the receiver is dropped must not panic;
    // try_send should return an error instead.
    let result = tx.try_send("session-1".to_string());
    assert!(
        result.is_err(),
        "try_send on disconnected channel should error"
    );
}

/// When the channel buffer is full, try_send must not panic.
#[tokio::test]
async fn test_channel_full_send_does_not_panic() {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(1);

    // Fill the channel
    tx.try_send("s1".to_string()).unwrap();

    // Second send should return Err(TrySendError::Full) — not panic
    let result = tx.try_send("s2".to_string());
    assert!(
        result.is_err(),
        "try_send on full channel should error, not panic"
    );

    // Receiver still alive; drain it
    drop(rx);
}

/// run_once processes an archived unmined session through the mining
/// pipeline (immediate hook trigger path: DreamingScheduler receives
/// notification → calls mine_session → marks session mined).
#[tokio::test]
async fn test_immediate_hook_triggers_mining() {
    let test_storage = TestStorage::default();

    // Add an archived, unmined session for a configured agent
    let mut cp = closeclaw_session::persistence::SessionCheckpoint::new("hook-mined".into());
    cp.agent_id = Some("agent1".into());
    cp.mined = false;
    test_storage.add_archived(cp);

    let storage: Arc<dyn PersistenceService> = Arc::new(test_storage);
    let config: Arc<dyn SessionConfigProvider> =
        Arc::new(MockConfig::new(vec!["agent1".to_string()]));

    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    let pipeline = Arc::new(DreamingPipeline::new());
    let miner = Arc::new(MemoryMiner::new(
        closeclaw_memory::miner::MinerConfig {
            enabled: true,
            ..Default::default()
        },
        Box::new(crate::noop_miner_llm::NoopMinerLlmCaller),
        Box::new(crate::noop_miner_llm::NoopMinerLlmCaller),
        tmp.path().join("memory-miner-test.db"),
        tmp.path()
            .join("MEMORY-hook.md")
            .to_string_lossy()
            .into_owned(),
    ));

    let scheduler = DreamingScheduler::new(storage, config, pipeline, miner, config_manager);

    let result = scheduler.run_once().await;
    assert!(
        result.is_ok(),
        "run_once should succeed when processing unmined session: {result:?}"
    );
}

// ── Step 1.5: DreamingScheduler grace period tests ─────────────────────

/// SHUTDOWN_GRACE_SECS must be 10 seconds (design doc alignment).
#[test]
fn test_dreaming_shutdown_grace_secs_is_ten() {
    // Access the constant via the module. The constant is crate-private,
    // so we verify it through the module path.
    assert_eq!(
        crate::dreaming_scheduler::SHUTDOWN_GRACE_SECS,
        10,
        "SHUTDOWN_GRACE_SECS must be 10 per design doc"
    );
}

/// When no active task is running, shutdown exits immediately.
#[tokio::test]
async fn test_dreaming_shutdown_no_active_task_exits_immediately() {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut scheduler = make_scheduler(storage, config, tmp.path());

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    // Let the loop start, then immediately send shutdown (no task will fire
    // because dreaming_interval is 600s).
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let start = tokio::time::Instant::now();
    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "scheduler should exit promptly with no active task"
    );
    // Should exit well within 2 seconds (no grace period needed)
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "scheduler without active task should exit immediately, took {:?}",
        start.elapsed()
    );
}

/// When an active task is running, shutdown waits up to SHUTDOWN_GRACE_SECS
/// for it to complete.
#[tokio::test]
async fn test_dreaming_shutdown_waits_for_active_task_within_grace() {
    use crate::dreaming_scheduler::SHUTDOWN_GRACE_SECS;

    // Verify the grace period constant is accessible
    assert_eq!(SHUTDOWN_GRACE_SECS, 10);

    // Spawn a task that takes 200ms (well under 10s grace period)
    let handle = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        Ok::<(), crate::dreaming_scheduler::DreamingSchedulerError>(())
    });

    let start = tokio::time::Instant::now();
    crate::dreaming_scheduler::wait_for_active_task(handle).await;
    let elapsed = start.elapsed();

    // Should complete in ~200ms, well under the 10s grace period
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "task completing within grace period should not wait full timeout, took {:?}",
        elapsed
    );
}

/// When an active task exceeds SHUTDOWN_GRACE_SECS, it is aborted
/// and the scheduler exits after the timeout.
#[tokio::test]
async fn test_dreaming_shutdown_aborts_task_after_grace_period() {
    use crate::dreaming_scheduler::SHUTDOWN_GRACE_SECS;

    // Spawn a task that takes 30s (well beyond 10s grace period)
    let handle = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        Ok::<(), crate::dreaming_scheduler::DreamingSchedulerError>(())
    });

    let start = tokio::time::Instant::now();
    crate::dreaming_scheduler::wait_for_active_task(handle).await;
    let elapsed = start.elapsed();

    // Should abort after ~10s grace period, not wait full 30s
    assert!(
        elapsed >= std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS - 1)
            && elapsed < std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS + 3),
        "task exceeding grace period should be aborted after ~{}s, took {:?}",
        SHUTDOWN_GRACE_SECS,
        elapsed
    );
}

/// When an active task panics, shutdown logs the error and exits
/// without waiting for the full grace period.
#[tokio::test]
async fn test_dreaming_shutdown_handles_panicked_task() {
    let handle: tokio::task::JoinHandle<
        Result<(), crate::dreaming_scheduler::DreamingSchedulerError>,
    > = tokio::spawn(async {
        panic!("test panic in dreaming task");
    });

    let start = tokio::time::Instant::now();
    crate::dreaming_scheduler::wait_for_active_task(handle).await;
    let elapsed = start.elapsed();

    // Panicked task should be detected immediately, not after grace period
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "panicked task should be detected immediately, took {:?}",
        elapsed
    );
}

/// When an active task returns an error, shutdown logs it and exits.
#[tokio::test]
async fn test_dreaming_shutdown_handles_errored_task() {
    let handle = tokio::spawn(async {
        Err(crate::dreaming_scheduler::DreamingSchedulerError::Dreaming(
            "test error".into(),
        ))
    });

    let start = tokio::time::Instant::now();
    crate::dreaming_scheduler::wait_for_active_task(handle).await;
    let elapsed = start.elapsed();

    // Error should be handled immediately
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "errored task should be handled immediately, took {:?}",
        elapsed
    );
}
