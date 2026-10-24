//! Shared test helpers for daemon tests.
//!
//! Duplicated from root crate's `common/test_helpers.rs` because the daemon
//! crate cannot depend on the root crate (circular dependency).

pub mod config;
pub mod dispatch;
pub mod fakes;
pub mod lifecycle;
pub mod shutdown;

pub use config::*;
pub(crate) use dispatch::*;
pub use fakes::*;
pub(crate) use lifecycle::*;
pub use shutdown::*;

use std::sync::Arc;
use std::time::Duration;

use closeclaw_common::CompactConfig;
use closeclaw_config::manager::ConfigSection;
use closeclaw_config::session::{PerAgentSessionConfig, SessionConfigProvider};
use closeclaw_config::ConfigManager;
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;

use closeclaw_session::persistence::{AgentRole, PersistenceService};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::dreaming_scheduler::DreamingScheduler;

// ── Shared DreamingScheduler test helpers ────────────────────────────────

/// Mock SessionConfigProvider that returns a configurable agent list.
#[derive(Debug)]
pub struct MockConfig {
    agents: Vec<String>,
    /// Fire-signal hook: when set, every `list_agents` call sends one
    /// message before returning (see [`MockConfig::with_on_list_agents`]).
    on_list_agents: Option<mpsc::UnboundedSender<()>>,
}

impl MockConfig {
    pub fn new(agents: Vec<String>) -> Self {
        Self {
            agents,
            on_list_agents: None,
        }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    /// Builder: install the fire-signal hook — every `list_agents` call
    /// then emits one message. `list_agents` is the first statement of
    /// every spawned dreaming cycle (`execute_dreaming_cycle`), so each
    /// message is the observable "a Fire happened" signal for loop-level
    /// regression tests — the loop-local `next_fire` itself is not
    /// observable from outside the loop (issue #3608). The channel
    /// rendezvous lets a test await each Fire individually: under
    /// `start_paused` the runtime auto-advances the virtual clock
    /// timer-by-timer while the test task is parked on `recv()`, so
    /// spawned cycles are polled (and their signals delivered) exactly
    /// when they run. Pair with an empty agent list: every cycle stays a
    /// no-op that finishes within its first poll, so cycles never overlap
    /// and one message == one Fire. Inject via
    /// [`make_config_change_scheduler_with_config`].
    pub fn with_on_list_agents(mut self, tx: mpsc::UnboundedSender<()>) -> Self {
        self.on_list_agents = Some(tx);
        self
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
        if let Some(fires) = &self.on_list_agents {
            let _ = fires.send(());
        }
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

/// ConfigManager over the given config directory, **without** skeleton
/// write or `load()` — the returned manager is unloaded and callers drive
/// the load/watch path themselves. Contrast [`make_config_manager`]: it
/// takes the TempDir root (config dir = `<root>/config`), writes the
/// mandatory skeleton, and calls `load()` before returning.
pub fn make_unloaded_config_manager(config_dir: &std::path::Path) -> Arc<ConfigManager> {
    Arc::new(
        ConfigManager::new(config_dir.to_path_buf()).expect("failed to create test ConfigManager"),
    )
}

// ── Config-change test helpers (setup / scenario / assert) ────────────

/// Setup: build a pipeline and miner with dreaming/mining DISABLED, so a
/// Memory section reload can be verified to flip them on (or stay off).
pub fn make_disabled_pipeline_and_miner(
    db_path: &std::path::Path,
    memory_md_path: &std::path::Path,
) -> (Arc<DreamingPipeline>, Arc<MemoryMiner>) {
    let pipeline = Arc::new(DreamingPipeline::with_config(
        closeclaw_memory::params::DreamingParams {
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
        db_path,
        memory_md_path.to_string_lossy().into_owned(),
    ));
    (pipeline, miner)
}

/// Setup: wire fresh storage and an empty mock config into a scheduler
/// around the given pipeline/miner/config manager.
pub fn make_config_change_scheduler(
    pipeline: Arc<DreamingPipeline>,
    miner: Arc<MemoryMiner>,
    config_manager: Arc<ConfigManager>,
) -> DreamingScheduler {
    make_config_change_scheduler_with_config(pipeline, miner, config_manager, MockConfig::empty())
}

/// Setup: [`make_config_change_scheduler`] over a caller-built
/// [`MockConfig`] — the injection point for mock hooks (e.g.
/// `with_on_list_agents` fire signalling), since the scheduler takes the
/// config as `Arc<dyn SessionConfigProvider>` and the mock cannot be
/// reached for wiring after it is wrapped.
pub fn make_config_change_scheduler_with_config(
    pipeline: Arc<DreamingPipeline>,
    miner: Arc<MemoryMiner>,
    config_manager: Arc<ConfigManager>,
    config: MockConfig,
) -> DreamingScheduler {
    let storage: Arc<dyn PersistenceService> = Arc::new(TestStorage::default());
    let config: Arc<dyn SessionConfigProvider> = Arc::new(config);
    DreamingScheduler::new(
        Arc::clone(&storage),
        Arc::new(crate::memory_storage_adapter::MemoryStorageAdapter::new(
            storage,
        )),
        config,
        pipeline,
        miner,
        config_manager,
    )
}

/// Setup: populate the Memory section cache with dreaming/mining enabled.
pub fn seed_memory_section_cache(config_manager: &ConfigManager) {
    seed_memory_section_cache_with_schedule(config_manager, None);
}

/// Setup: [`seed_memory_section_cache`] plus a `dreaming.schedule` value
/// — variant for the cron-loop reload tests (issue #3581), which need
/// the schedule the reloaded Memory section carries (the schedule the
/// cron loop re-parses in `handle_cron_config_reload`).
pub fn seed_memory_section_cache_with_schedule(
    config_manager: &ConfigManager,
    schedule: Option<&str>,
) {
    let mut memory_value = serde_json::json!({
        "dreaming": {
            "enabled": true
        },
        "mining": {
            "enabled": true
        }
    });
    if let Some(schedule) = schedule {
        memory_value["dreaming"]["schedule"] = serde_json::json!(schedule);
    }
    config_manager.update_section_cache(
        closeclaw_config::ConfigSection::Memory,
        std::path::PathBuf::from("memory.json"),
        memory_value,
    );
}

/// Scenario: broadcast a section reload event.
///
/// Synchronous by design: the scheduler's receiver subscribed at
/// construction (`DreamingScheduler::new` → `subscribe_config_changes`),
/// so the event is guaranteed buffered in the receiver — no settle sleep
/// is needed. Consumers observe the effect deterministically via
/// [`wait_until`] (plan Step 1.5: the previous fixed 50ms sleep was
/// redundant and removed).
pub fn notify_section_reload(config_manager: &ConfigManager, section: ConfigSection, path: &str) {
    config_manager.notify_change(closeclaw_config::ConfigChangeEvent::Reloaded {
        section,
        path: path.into(),
    });
}

/// Poll `ready` until it holds, failing the test if it doesn't within 1s.
///
/// Deterministic substitute for fixed sleeps: waits on an observable state
/// flip (e.g. config taking effect) with a deadline instead of guessing a
/// wall-clock window (STANDARDS §9: 确定性优先).
pub async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out after 1s waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Scenario: run the scheduler loop, wait for `event_consumed` — the
/// caller's signal that the pending config event has been received and
/// applied (an observable state flip; or a bounded window when the event
/// intentionally produces no observable change) — then shut the loop down
/// and verify prompt, panic-free exit.
///
/// `what` describes the scenario under test and is surfaced in the
/// assertion message when the shutdown wait fails (same pattern as
/// [`wait_until`]).
///
/// Returns the value produced by `event_consumed`, so callers can
/// observe data gathered while waiting (e.g. elapsed virtual time).
pub async fn run_scheduler_briefly<T>(
    mut scheduler: DreamingScheduler,
    event_consumed: impl std::future::Future<Output = T>,
    what: &str,
) -> T {
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move {
        scheduler.run(shutdown_rx).await;
    });

    let output = event_consumed.await;
    shutdown_tx.send(()).unwrap();

    let joined = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(joined.is_ok(), "scheduler should exit promptly: {what}");
    assert!(
        joined.expect("timeout already asserted above").is_ok(),
        "scheduler task must not panic: {what}"
    );
    output
}
