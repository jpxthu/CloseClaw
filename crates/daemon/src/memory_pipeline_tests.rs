//! Unit tests for the memory pipeline assembly
//! [`Daemon::spawn_memory_pipeline`] (extracted from
//! `spawn_background_services`, issue #3286 Step 1.1).
//!
//! Behavior dimensions (plan Step 1.5):
//! 1. Normal path: `memory.json` fully configured (mining enabled, custom
//!    storage paths, relaxed clean rules) → the pipeline is assembled from
//!    that config: an archive mining notification drives the MemoryMiner
//!    to create its sqlite DB at the **configured** path and mark the
//!    session mined in storage.
//! 2. Boundary (config absent → degraded defaults): without any
//!    `memory.json` the assembly falls back to defaults — mining disabled
//!    — so the notify is consumed by the scheduler loop without any
//!    mining side effect and the scheduler still shuts down cleanly.
//! 3. Boundary (storage section absent → default paths): `memory.json`
//!    with only `mining.enabled` → the miner falls back to the hardcoded
//!    default DB path `memory/memory.db` instead of panicking.

use std::sync::Arc;
use std::time::Duration;

use closeclaw_common::test_helpers::ensure_dir;
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_session::persistence::{PendingMessage, PersistenceService, SessionCheckpoint};
use tokio::sync::{mpsc, watch};

use crate::test_helpers::{make_config_manager, wait_until, MockConfig, TestStorage};
use crate::Daemon;

/// Write `memory.json` into `<root>/config` **before** the shared
/// [`make_config_manager`] constructor runs `load()`, mirroring the
/// production layout (global memory config lives next to the mandatory
/// section files).
fn write_memory_json(root: &std::path::Path, json: &str) {
    let config_dir = root.join("config");
    ensure_dir(&config_dir).expect("create config dir");
    std::fs::write(config_dir.join("memory.json"), json).expect("write memory.json");
}

/// Seed one archived, unmined session for `agent_id` whose pending
/// messages form a minable transcript (`user: …` lines satisfy the
/// owner-message count).
fn seed_archived_session(storage: &TestStorage, session_id: &str, agent_id: &str) {
    let mut cp = SessionCheckpoint::new(session_id.to_string());
    cp.agent_id = Some(agent_id.to_string());
    cp.mined = false;
    cp.outbound_pending = vec![
        PendingMessage::with_role("m1".into(), "hello".into(), "user".into()),
        PendingMessage::with_role("m2".into(), "hi there".into(), "assistant".into()),
    ];
    storage.add_archived(cp);
}

/// Assemble the pipeline over the given config manager / data dir and
/// drive one mining notification through it. Returns the scheduler task
/// handle (shutdown: drop the senders, then await) and the shared test
/// storage.
struct PipelineFixture {
    handle: tokio::task::JoinHandle<()>,
    storage: Arc<TestStorage>,
    mining_notify_tx: mpsc::Sender<String>,
    dreaming_tx: watch::Sender<()>,
}

fn spawn_pipeline(
    config_manager: &Arc<closeclaw_config::ConfigManager>,
    data_dir: &std::path::Path,
    config: Arc<dyn SessionConfigProvider>,
    storage: Arc<TestStorage>,
) -> PipelineFixture {
    let storage_dyn: Arc<dyn PersistenceService> = Arc::clone(&storage) as _;
    let (mining_notify_tx, mining_notify_rx) = mpsc::channel(32);
    let (dreaming_tx, dreaming_rx) = watch::channel(());
    let handle = Daemon::spawn_memory_pipeline(
        config_manager,
        &storage_dyn,
        data_dir,
        config,
        mining_notify_rx,
        dreaming_rx,
    );
    PipelineFixture {
        handle,
        storage,
        mining_notify_tx,
        dreaming_tx,
    }
}

/// Shut the pipeline down deterministically: the loop handles events
/// serially, so once the shutdown senders are dropped the task must exit
/// promptly and without panicking.
async fn shutdown_and_join(fixture: PipelineFixture) {
    let PipelineFixture {
        handle,
        mining_notify_tx,
        dreaming_tx,
        ..
    } = fixture;
    drop(mining_notify_tx);
    drop(dreaming_tx);
    let joined = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(joined.is_ok(), "scheduler should exit promptly on shutdown");
    assert!(
        joined.expect("timeout already asserted above").is_ok(),
        "scheduler task must not panic"
    );
}

// ── Normal path: memory.json complete → assembled per config ────────────

/// With a complete memory config the notify drives the assembled
/// pipeline end to end: the miner is constructed with the config's
/// `storage.dbPath` (DB file appears there) and the session is marked
/// mined — proving config → component wiring, not just spawn success.
#[tokio::test]
async fn test_spawn_memory_pipeline_uses_memory_config() {
    let tmp = tempfile::tempdir().expect("temp dir");
    write_memory_json(
        tmp.path(),
        r#"{
            "mining": {
                "enabled": true,
                "transcriptCleanRules": { "minTurns": 1, "minOwnerMsgs": 1 }
            },
            "storage": { "dbPath": "custom_mem.db", "memoryMdPath": "custom_MEMORY.md" }
        }"#,
    );
    let config_manager = make_config_manager(tmp.path());

    let storage = Arc::new(TestStorage::default());
    seed_archived_session(&storage, "s1", "test-agent");
    let config: Arc<dyn SessionConfigProvider> =
        Arc::new(MockConfig::new(vec!["test-agent".to_string()]));

    let fixture = spawn_pipeline(&config_manager, tmp.path(), config, Arc::clone(&storage));
    assert!(
        !fixture.handle.is_finished(),
        "scheduler task must be running after spawn"
    );

    fixture
        .mining_notify_tx
        .send("s1".to_string())
        .await
        .expect("send mining notification");

    wait_until("miner DB at the configured custom path", || {
        tmp.path().join("custom_mem.db").exists()
    })
    .await;
    wait_until("session marked mined in storage", || {
        fixture.storage.mined_ids().contains(&"s1".to_string())
    })
    .await;

    shutdown_and_join(fixture).await;
}

// ── Boundary: memory.json absent → degraded defaults ────────────────────

/// Without memory.json the assembly degrades to defaults: mining is
/// disabled, so the notification is consumed without any mining side
/// effect (no default-path DB, nothing marked mined) and the scheduler
/// shuts down cleanly. The unbounded fire-signal channel makes the
/// consumption deterministic: `list_agents` is the first statement of
/// the notify handler, and the loop handles events serially, so a clean
/// join proves the disabled-mining notify was fully processed.
#[tokio::test]
async fn test_spawn_memory_pipeline_missing_config_defaults_no_mining() {
    let tmp = tempfile::tempdir().expect("temp dir");
    // Mandatory skeleton only — no memory.json.
    let config_manager = make_config_manager(tmp.path());

    let storage = Arc::new(TestStorage::default());
    seed_archived_session(&storage, "s1", "test-agent");

    let (fire_tx, mut fire_rx) = mpsc::unbounded_channel();
    let config: Arc<dyn SessionConfigProvider> =
        Arc::new(MockConfig::new(vec!["test-agent".to_string()]).with_on_list_agents(fire_tx));

    let fixture = spawn_pipeline(&config_manager, tmp.path(), config, Arc::clone(&storage));

    fixture
        .mining_notify_tx
        .send("s1".to_string())
        .await
        .expect("send mining notification");
    fire_rx
        .recv()
        .await
        .expect("notify handler must reach list_agents");

    shutdown_and_join(fixture).await;

    assert!(
        !tmp.path().join("memory").join("memory.db").exists(),
        "mining disabled by default → no DB at the default path"
    );
    assert!(
        storage.mined_ids().is_empty(),
        "mining disabled by default → session never marked mined"
    );
}

// ── Boundary: storage section absent → default DB path ──────────────────

/// `memory.json` that enables mining but omits the `storage` section →
/// the miner falls back to the hardcoded default DB path
/// (`memory/memory.db`) instead of failing: with the parent directory in
/// place the mining run creates the DB there and marks the session mined.
#[tokio::test]
async fn test_spawn_memory_pipeline_default_db_path_when_storage_section_missing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    write_memory_json(
        tmp.path(),
        r#"{
            "mining": {
                "enabled": true,
                "transcriptCleanRules": { "minTurns": 1, "minOwnerMsgs": 1 }
            }
        }"#,
    );
    // The default db_path is relative ("memory/memory.db") — its parent
    // must exist for sqlite to open the file (production provides it via
    // the data-dir layout).
    ensure_dir(&tmp.path().join("memory")).expect("create default memory dir");
    let config_manager = make_config_manager(tmp.path());

    let storage = Arc::new(TestStorage::default());
    seed_archived_session(&storage, "s1", "test-agent");
    let config: Arc<dyn SessionConfigProvider> =
        Arc::new(MockConfig::new(vec!["test-agent".to_string()]));

    let fixture = spawn_pipeline(&config_manager, tmp.path(), config, Arc::clone(&storage));

    fixture
        .mining_notify_tx
        .send("s1".to_string())
        .await
        .expect("send mining notification");

    wait_until("miner DB at the default path", || {
        tmp.path().join("memory").join("memory.db").exists()
    })
    .await;
    wait_until("session marked mined in storage", || {
        fixture.storage.mined_ids().contains(&"s1".to_string())
    })
    .await;

    shutdown_and_join(fixture).await;
}
