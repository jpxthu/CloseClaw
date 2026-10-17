//! Unit tests for the shared test helpers:
//!
//! - the narrowed `kill_self` interface (issue #3140): [`TestShutdownSignal`]
//!   carries the SIGTERM/SIGINT-only invariant in code, so these tests pin
//!   both the type-level variant → libc encoding and the end-to-end delivery
//!   each variant promises to its callers.
//! - the [`run_scheduler_briefly`] shutdown scaffolding (issue #3708): the
//!   normal path returns after the shutdown signal, and the `what` context
//!   surfaces in the panic message when the 5s exit deadline is missed.
//!
//! Placement (plan Step 1.3): this dedicated helpers-test file (the #3140
//! precedent) instead of inline next to the helper — `test_helpers.rs` sits
//! near the 1000-line hard limit (CONTRIBUTING §硬性限制).

use crate::dreaming_scheduler::DreamingScheduler;
use crate::test_helpers::{
    kill_self, make_config_change_scheduler, make_disabled_pipeline_and_miner,
    make_test_config_manager, run_scheduler_briefly, MockConfig, TestShutdownSignal,
};
use async_trait::async_trait;
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

/// Locks the variant → libc signal-number encoding: `Sigterm` must be
/// `libc::SIGTERM`, `Sigint` must be `libc::SIGINT`. A swapped mapping
/// would silently change which signal every shutdown test delivers.
#[test]
fn test_shutdown_signal_variant_maps_to_libc_signal() {
    assert_eq!(
        TestShutdownSignal::Sigterm.as_libc_signal(),
        libc::SIGTERM,
        "Sigterm variant must encode libc::SIGTERM"
    );
    assert_eq!(
        TestShutdownSignal::Sigint.as_libc_signal(),
        libc::SIGINT,
        "Sigint variant must encode libc::SIGINT"
    );
}

/// Delivers each variant with `kill_self` to a handler registered only for
/// its expected kind, so only a correctly mapped signal can wake it. Pins
/// the full chain variant → `as_libc_signal` → `libc::kill` → kernel →
/// handler; the existing shutdown cases use symmetric dual-branch selects
/// and cannot tell the two signals apart.
/// serial: signals are process-wide (STANDARDS §7); the 10s timeout is an
/// upper bound only — success path completes on delivery (STANDARDS §6 §9).
#[serial_test::serial]
#[tokio::test]
async fn test_kill_self_shutdown_signal_reaches_matching_handler() {
    // Independent oracle: which handler must observe each variant.
    let cases = [
        (TestShutdownSignal::Sigterm, SignalKind::terminate()),
        (TestShutdownSignal::Sigint, SignalKind::interrupt()),
    ];
    for (sig, kind) in cases {
        // Registration strictly precedes the send, so delivery is safe.
        let mut handler = signal(kind).expect("register signal handler");
        kill_self(sig);
        tokio::time::timeout(Duration::from_secs(10), handler.recv())
            .await
            .unwrap_or_else(|_| {
                panic!("kill_self({sig:?}) was not observed by its matching handler within 10s")
            })
            .expect("matching handler stream ended without delivering the signal");
    }
}

// ── run_scheduler_briefly helper behavior (issue #3708) ───────────────

/// Minimal [`PersistenceService`] whose `list_archived_unmined_sessions`
/// never resolves: a dreaming cycle with a configured agent parks there
/// forever, so the scheduler's shutdown epilogue must wait on a stuck
/// active task — the setup that drives [`run_scheduler_briefly`] into its
/// 5s timeout branch with zero real waiting.
struct HangingUnminedStorage;

#[async_trait]
impl PersistenceService for HangingUnminedStorage {
    async fn save_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        std::future::pending().await
    }
}

/// Normal path: after `event_consumed` completes, the shutdown signal is
/// delivered and the scheduler loop joins within the helper's 5s deadline
/// — the helper returns normally (issue #3708 Step 1.3). Paused clock: the
/// watch-channel shutdown wakeup needs no timer, so the whole test runs in
/// zero virtual time and cannot pass merely by racing the deadline.
#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn test_run_scheduler_briefly_returns_after_shutdown() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-helper-normal.db"),
        &tmp.path().join("MEMORY-helper-normal.md"),
    );
    let scheduler = make_config_change_scheduler(pipeline, miner, config_manager);

    run_scheduler_briefly(scheduler, std::future::ready(()), "helper normal path").await;
}

/// Timeout path: when the scheduler cannot exit, the helper must panic
/// with the `scheduler should exit promptly: {what}` diagnostic — the
/// `what` context travels onto the assertion failure path (issue #3708
/// Step 1.3). The stuck cycle pins the shutdown epilogue in its grace
/// wait, and under the paused clock the helper's virtual 5s deadline fires
/// first (before the 10s grace), so the failure is simulated with zero
/// real elapsed time — no real 5s wait (CI >5s red line).
#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn test_run_scheduler_briefly_panics_with_what_context_on_timeout() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-helper-timeout.db"),
        &tmp.path().join("MEMORY-helper-timeout.md"),
    );
    let storage: Arc<dyn PersistenceService> = Arc::new(HangingUnminedStorage);
    let (fire_tx, mut fire_rx) = mpsc::unbounded_channel();
    let scheduler = DreamingScheduler::new(
        Arc::clone(&storage),
        Arc::new(crate::memory_storage_adapter::MemoryStorageAdapter::new(
            storage,
        )),
        Arc::new(MockConfig::new(vec!["agent".to_string()]).with_on_list_agents(fire_tx)),
        pipeline,
        miner,
        config_manager,
    );

    // The fire signal is `list_agents` starting the first cycle, i.e. the
    // proof the (now-stuck) active task exists before shutdown is sent.
    // Virtual horizon (await_n_fires precedent, #3608): under the paused
    // clock the recv completes at ~zero virtual time, while a loop that
    // never fires fails the horizon assert instead of hanging the test —
    // the deadline is virtual, so real elapsed time stays milliseconds.
    let joined = tokio::spawn(async move {
        run_scheduler_briefly(
            scheduler,
            async {
                tokio::time::timeout(Duration::from_secs(3600), fire_rx.recv())
                    .await
                    .expect("fire signal must arrive within the virtual horizon")
                    .expect("fire signal channel stays open while the loop runs");
            },
            "helper timeout diagnostic",
        )
        .await
    })
    .await;
    let join_err =
        joined.expect_err("helper must panic when the scheduler misses the 5s exit deadline");
    assert!(
        join_err.is_panic(),
        "expected a panic JoinError, got: {join_err:?}"
    );
    let payload = join_err.into_panic();
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("panic payload should be a string");
    assert!(
        msg.contains("scheduler should exit promptly: helper timeout diagnostic"),
        "expected the prompt-exit diagnostic carrying the what label, got: {msg}"
    );
}
