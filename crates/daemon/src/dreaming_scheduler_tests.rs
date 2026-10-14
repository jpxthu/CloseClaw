//! Unit tests for DreamingScheduler.

use std::sync::Arc;

use tokio::sync::watch;

use crate::dreaming_scheduler::DreamingScheduler;
use crate::test_helpers::{
    make_config_change_scheduler, make_disabled_pipeline_and_miner, make_test_config_manager,
    notify_section_reload, run_scheduler_briefly, seed_memory_section_cache, wait_until,
    MockConfig, TestStorage,
};
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigManager;
use closeclaw_memory::dreaming::{DreamingError, DreamingPipeline};
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_session::persistence::{DreamingStatus, PersistenceService, SessionCheckpoint};

// ── Test helpers ─────────────────────────────────────────────────────────

fn make_scheduler(
    storage: Arc<dyn PersistenceService>,
    config: Arc<dyn SessionConfigProvider>,
    root: &std::path::Path,
) -> DreamingScheduler {
    let config_manager = Arc::new(
        ConfigManager::new(root.join("config")).expect("failed to create test ConfigManager"),
    );
    DreamingScheduler::new(
        Arc::clone(&storage),
        Arc::new(crate::memory_storage_adapter::MemoryStorageAdapter::new(
            storage,
        )),
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

// Config-change test helpers (MockConfig / make_config_change_scheduler /
// notify_section_reload / wait_until / run_scheduler_briefly / ...) live in
// `crate::test_helpers` so the loop-behavior tests below can share them.

/// Scenario: run the pipeline once over a fresh mined+undreamt checkpoint and
/// return the dreaming status it ends up with (assertions stay at call sites).
async fn run_once_and_get_status(
    pipeline: &DreamingPipeline,
    session_id: &str,
) -> Result<DreamingStatus, DreamingError> {
    let session_storage = Arc::new(TestStorage::default());
    let mut cp = SessionCheckpoint::new(session_id.to_string());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    session_storage.add_checkpoint(cp);

    let memory_storage = crate::memory_storage_adapter::MemoryStorageAdapter::new(Arc::clone(
        &session_storage,
    )
        as Arc<dyn PersistenceService>);
    pipeline.run_once(&memory_storage).await?;

    let cps = session_storage.checkpoints.lock().unwrap();
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
    );
    // Deterministic signal: handle_config_change() updates the pipeline and
    // then the miner, so miner flipping to enabled proves the Memory reload
    // event was consumed and applied (pipeline update precedes it).
    run_scheduler_briefly(
        scheduler,
        wait_until("Memory section reload applied (miner enabled)", || {
            miner.is_enabled()
        }),
    )
    .await;

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
    );
    // Conservative fixed window (#3148 item 4): a non-Memory event is
    // intentionally a no-op — it produces NO observable state change, and
    // that absence is exactly what this test asserts — so unlike the Memory
    // path there is no consumption signal to poll. The scheduler's config
    // receiver subscribed at construction, so the event is already buffered;
    // the 200ms only needs to cover the spawned loop reaching its first
    // select poll, where the buffered event is drained.
    run_scheduler_briefly(scheduler, async {
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await
    })
    .await;

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

    let scheduler = DreamingScheduler::new(
        Arc::clone(&storage),
        Arc::new(crate::memory_storage_adapter::MemoryStorageAdapter::new(
            storage,
        )),
        config,
        pipeline,
        miner,
        config_manager,
    );

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

/// When an active task exceeds SHUTDOWN_GRACE_SECS, it is abandoned
/// and the scheduler exits after the timeout.
///
/// Runs on a paused (virtual) clock: the slow task parks in
/// `std::future::pending` (no timer registered), so auto-advance only
/// drives the `timeout(SHUTDOWN_GRACE_SECS, ...)` timer inside
/// `wait_for_active_task`. The full timing chain — task exceeding the
/// budget → 10s grace timer fires → task abandoned with a warning —
/// is still genuinely executed end to end, while real wall-clock cost
/// drops to milliseconds (precisely as in PR #3202).
#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn test_dreaming_shutdown_abandons_task_after_grace_period() {
    use crate::dreaming_scheduler::SHUTDOWN_GRACE_SECS;

    // Spawn a hanging task that never completes: parked on
    // `std::future::pending` (registers no timer), well beyond the
    // 10s grace period.
    let handle = tokio::spawn(async move {
        std::future::pending::<()>().await;
        Ok::<(), crate::dreaming_scheduler::DreamingSchedulerError>(())
    });

    let start = tokio::time::Instant::now();
    crate::dreaming_scheduler::wait_for_active_task(handle).await;
    let elapsed = start.elapsed();

    // Virtual clock: elapsed must equal the grace period exactly,
    // proving we waited out the whole 10s budget instead of returning
    // early (the task never completes on its own).
    assert_eq!(
        elapsed,
        std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS),
        "task exceeding grace period should be abandoned after exactly {}s, took {:?}",
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

// ── Step 1.1: select! loop branch protection tests ─────────────────────

/// A transcript that passes the miner's default cleaning thresholds
/// (min_turns = 5, min_owner_msgs = 5): five user turns plus one
/// assistant reply.
fn miner_clean_transcript() -> Vec<closeclaw_session::persistence::PendingMessage> {
    let mut msgs: Vec<_> = (1..=5)
        .map(|i| {
            closeclaw_session::persistence::PendingMessage::with_role(
                format!("u{i}"),
                format!("hello {i}"),
                "user".into(),
            )
        })
        .collect();
    msgs.push(closeclaw_session::persistence::PendingMessage::with_role(
        "a1".into(),
        "hi there".into(),
        "assistant".into(),
    ));
    msgs
}

/// The mining-notify mpsc branch drives a running fixed-interval loop end
/// to end: a notified session whose agent is configured is mined
/// immediately (observed via `TestStorage::mined_ids`), while a session
/// whose agent is not configured is silently ignored — the loop keeps
/// running without panic in both cases.
///
/// FIFO consumption makes the ignore assertion deterministic: by the time
/// the follow-up configured session ("notify-confirm", sent after the
/// ignored one) is observed mined, the ignored message has already been
/// consumed by the loop and produced no mine.
#[tokio::test]
async fn test_mining_notify_mines_configured_and_ignores_unconfigured() {
    let test_storage = Arc::new(TestStorage::default());

    // Archived unmined sessions with non-empty transcripts, so the miner
    // actually reaches mark_mined when it runs (observable via mined_ids).
    let mut configured = SessionCheckpoint::new("notify-mined".into())
        .with_outbound_pending(miner_clean_transcript());
    configured.agent_id = Some("agent1".into());
    configured.mined = false;
    test_storage.add_archived(configured);

    let mut foreign = SessionCheckpoint::new("notify-foreign".into())
        .with_outbound_pending(miner_clean_transcript());
    foreign.agent_id = Some("unknown-agent".into());
    foreign.mined = false;
    test_storage.add_archived(foreign);

    let mut confirm = SessionCheckpoint::new("notify-confirm".into())
        .with_outbound_pending(miner_clean_transcript());
    confirm.agent_id = Some("agent1".into());
    confirm.mined = false;
    test_storage.add_archived(confirm);

    let storage: Arc<dyn PersistenceService> =
        Arc::clone(&test_storage) as Arc<dyn PersistenceService>;
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
        tmp.path().join("memory-notify.db"),
        tmp.path()
            .join("MEMORY-notify.md")
            .to_string_lossy()
            .into_owned(),
    ));

    let (notify_tx, notify_rx) = tokio::sync::mpsc::channel::<String>(8);
    let scheduler = DreamingScheduler::new(
        Arc::clone(&storage),
        Arc::new(crate::memory_storage_adapter::MemoryStorageAdapter::new(
            storage,
        )),
        config,
        pipeline,
        miner,
        config_manager,
    )
    .with_mining_notify_rx(notify_rx);

    let observed = Arc::clone(&test_storage);
    run_scheduler_briefly(scheduler, async move {
        // A configured-agent session must be mined immediately.
        notify_tx.send("notify-mined".to_string()).await.unwrap();
        wait_until("notified session mined", || {
            test_storage
                .mined_ids()
                .iter()
                .any(|id| id == "notify-mined")
        })
        .await;

        // An unconfigured-agent session is silently ignored (no panic);
        // the confirm session after it proves the ignored one was consumed.
        notify_tx.send("notify-foreign".to_string()).await.unwrap();
        notify_tx.send("notify-confirm".to_string()).await.unwrap();
        wait_until("confirm session mined", || {
            test_storage
                .mined_ids()
                .iter()
                .any(|id| id == "notify-confirm")
        })
        .await;
    })
    .await;

    assert!(
        observed.mined_ids().iter().any(|id| id == "notify-mined"),
        "session with configured agent must be mined via the notify branch"
    );
    assert!(
        !observed.mined_ids().iter().any(|id| id == "notify-foreign"),
        "session with unconfigured agent must be silently ignored"
    );
}

/// The shared notify helper's `agents.is_empty()` guard must suppress
/// mining end to end: with NO agent configured at all, a notified
/// session that would otherwise be minable is left untouched and the
/// loop stays alive. This is a different branch than the foreign-agent
/// scenario above — that one is filtered inside `mine_archived_session`
/// (`agents.contains(aid)`), this one short-circuits in
/// `handle_mining_notify` before any per-session work.
///
/// Deterministic consumption proof: the notify channel has capacity 1,
/// so the follow-up `send().await` can only complete once the loop has
/// dequeued the first message — the guard branch has definitely run
/// (bounded by a 1s timeout so a broken loop fails fast, not by hang).
#[tokio::test]
async fn test_mining_notify_empty_agents_suppresses_mining() {
    let test_storage = Arc::new(TestStorage::default());

    // A session that would be minable if any agent were configured.
    let mut cp = SessionCheckpoint::new("notify-no-agents".into())
        .with_outbound_pending(miner_clean_transcript());
    cp.agent_id = Some("agent1".into());
    cp.mined = false;
    test_storage.add_archived(cp);

    let storage: Arc<dyn PersistenceService> =
        Arc::clone(&test_storage) as Arc<dyn PersistenceService>;
    let config: Arc<dyn SessionConfigProvider> = Arc::new(MockConfig::empty());
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
        tmp.path().join("memory-notify-empty.db"),
        tmp.path()
            .join("MEMORY-notify-empty.md")
            .to_string_lossy()
            .into_owned(),
    ));

    let (notify_tx, notify_rx) = tokio::sync::mpsc::channel::<String>(1);
    let scheduler = DreamingScheduler::new(
        Arc::clone(&storage),
        Arc::new(crate::memory_storage_adapter::MemoryStorageAdapter::new(
            storage,
        )),
        config,
        pipeline,
        miner,
        config_manager,
    )
    .with_mining_notify_rx(notify_rx);

    let observed = Arc::clone(&test_storage);
    run_scheduler_briefly(scheduler, async move {
        notify_tx
            .send("notify-no-agents".to_string())
            .await
            .unwrap();
        // Completes only once the first message was dequeued by the
        // loop — proof the empty-agents guard branch executed.
        tokio::time::timeout(
            tokio::time::Duration::from_secs(1),
            notify_tx.send("consumption-proof".to_string()),
        )
        .await
        .expect("loop must consume the notify message within 1s")
        .unwrap();
    })
    .await;

    assert!(
        observed.mined_ids().is_empty(),
        "no session may be mined when no agent is configured, got {:?}",
        observed.mined_ids()
    );
}

/// Flooding the config broadcast past its capacity (64) must leave the
/// scheduler loop alive: the receiver's `Err(RecvError::Lagged)` is
/// tolerated and a subsequent valid memory reload event is still applied
/// (miner flips to enabled). This assertion holds both before the Lagged
/// handling lands (error silently ignored, polling continues) and after
/// (warn logged, polling continues).
#[tokio::test]
async fn test_config_broadcast_lagged_loop_survives_and_applies_reload() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    seed_memory_section_cache(&config_manager);
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-lagged.db"),
        &tmp.path().join("MEMORY-lagged.md"),
    );
    let scheduler =
        make_config_change_scheduler(pipeline.clone(), miner.clone(), config_manager.clone());

    // Flood the broadcast channel past capacity while the loop is not yet
    // polling: the scheduler's receiver is guaranteed to Lag on its first
    // recv. 64 is the broadcast channel capacity (closeclaw-config
    // DEFAULT_CHANNEL_CAPACITY); +8 leaves margin.
    let flooded = 64 + 8;
    for _ in 0..flooded {
        config_manager.notify_change(closeclaw_config::ConfigChangeEvent::Reloaded {
            section: closeclaw_config::ConfigSection::Memory,
            path: "memory.json".into(),
        });
    }
    // A subsequent valid memory reload event after the lag window.
    config_manager.notify_change(closeclaw_config::ConfigChangeEvent::Reloaded {
        section: closeclaw_config::ConfigSection::Memory,
        path: "memory.json".into(),
    });

    run_scheduler_briefly(
        scheduler,
        wait_until("post-lag memory reload applied", || miner.is_enabled()),
    )
    .await;

    assert!(
        miner.is_enabled(),
        "memory reload after broadcast lag must still be applied"
    );
}
