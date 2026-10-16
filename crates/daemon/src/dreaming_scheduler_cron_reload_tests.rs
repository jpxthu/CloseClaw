//! Unit tests for the DreamingScheduler cron-loop config-reload path
//! (issue #3581): `handle_cron_config_reload` — schedule switching,
//! next-fire recomputation from the reloaded schedule, invalid-schedule
//! fallback to the fixed interval, and non-Memory reloads being ignored
//! by the cron loop.
//!
//! Also houses the loop-level Fire-persistence regression tests for
//! issue #3608: after a reload, every subsequent Fire must re-arm on the
//! currently loaded schedule (or the fixed-interval fallback), not
//! silently revert to the schedule the loop started with.
//!
//! Own file (not appended to `dreaming_scheduler_tests.rs`) because that
//! file sits near the 1000-line hard limit (CONTRIBUTING §硬性限制).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use crate::test_helpers::{
    make_config_change_scheduler, make_config_change_scheduler_with_config,
    make_disabled_pipeline_and_miner, make_test_config_manager, notify_section_reload,
    run_scheduler_briefly, seed_memory_section_cache_with_schedule, wait_until, MockConfig,
};
use closeclaw_config::ConfigSection;

// ── Normal path: schedule switch + next_fire recomputation ────────────

/// Normal path: a Memory-section reload carrying a valid new cron
/// schedule switches `schedule` and recomputes `next_fire` from the new
/// schedule (issue #3581).
///
/// Drives `handle_cron_config_reload` directly so the loop-local
/// `next_fire` is observable: it starts at a distant horizon (as the old
/// daily-at-3am schedule's next fire would be) and must land on the new
/// every-minute schedule's next whole-minute boundary — strictly in the
/// future, within 60s of the reload.
#[tokio::test]
async fn test_cron_config_reload_switches_schedule_and_recomputes_next_fire() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    seed_memory_section_cache_with_schedule(&config_manager, Some("* * * * *"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-cron-reload.db"),
        &tmp.path().join("MEMORY-cron-reload.md"),
    );
    let mut scheduler = make_config_change_scheduler(pipeline, miner, config_manager)
        .with_schedule(Some("0 3 * * *".to_string()));

    let before = tokio::time::Instant::now();
    let mut next_fire = before + Duration::from_secs(600);

    scheduler.handle_cron_config_reload(&mut next_fire).await;

    assert_eq!(
        scheduler.schedule.as_deref(),
        Some("* * * * *"),
        "reload must switch the scheduler to the reloaded cron schedule"
    );
    assert!(
        next_fire > before,
        "recomputed next_fire must be in the future: {next_fire:?} vs {before:?}"
    );
    assert!(
        next_fire <= before + Duration::from_secs(60),
        "every-minute schedule must fire within 60s of the reload, got {:?} after",
        next_fire - before
    );
}

/// Loop-driven normal path: the cron loop consumes a Memory-section
/// reload and drives `handle_cron_config_reload` (whose first step,
/// `handle_config_change`, applies the new config): the seeded disabled
/// miner flips to enabled — the deterministic consumption proof
/// (issue #3581).
#[tokio::test]
async fn test_cron_loop_applies_memory_reload() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    seed_memory_section_cache_with_schedule(&config_manager, Some("* * * * *"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-cron-loop-reload.db"),
        &tmp.path().join("MEMORY-cron-loop-reload.md"),
    );
    let scheduler =
        make_config_change_scheduler(pipeline.clone(), miner.clone(), config_manager.clone())
            .with_schedule(Some("0 3 * * *".to_string()));

    notify_section_reload(&config_manager, ConfigSection::Memory, "memory.json");

    run_scheduler_briefly(
        scheduler,
        wait_until("cron loop applied Memory reload", || miner.is_enabled()),
    )
    .await;

    assert!(
        miner.is_enabled(),
        "cron loop must apply the Memory reload via handle_cron_config_reload"
    );
}

// ── Error/fallback path: invalid schedule → fixed interval ────────────

/// Error/fallback path: after a reload the schedule becomes invalid —
/// the empty string and a non-cron expression are both covered — the
/// handler still stores the raw value, but the loop falls back to the
/// fixed interval: `next_fire = now + dreaming_interval_secs` (600s in
/// MockConfig), not a cron-derived instant (issue #3581).
#[tokio::test]
async fn test_cron_config_reload_invalid_schedule_falls_back_to_fixed_interval() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-cron-fallback.db"),
        &tmp.path().join("MEMORY-cron-fallback.md"),
    );
    let mut scheduler = make_config_change_scheduler(pipeline, miner, Arc::clone(&config_manager))
        .with_schedule(Some("0 3 * * *".to_string()));

    for invalid in ["", "not-a-cron"] {
        seed_memory_section_cache_with_schedule(&config_manager, Some(invalid));
        let before = tokio::time::Instant::now();
        let mut next_fire = before;
        scheduler.handle_cron_config_reload(&mut next_fire).await;
        let after = tokio::time::Instant::now();

        assert_eq!(
            scheduler.schedule.as_deref(),
            Some(invalid),
            "reload must store the reloaded schedule verbatim"
        );
        let fixed_lo = before + Duration::from_secs(600);
        let fixed_hi = after + Duration::from_secs(600);
        assert!(
            next_fire >= fixed_lo && next_fire <= fixed_hi,
            "invalid schedule {invalid:?} must fall back to fixed interval 600s, \
             got {next_fire:?} (window {fixed_lo:?}..{fixed_hi:?})"
        );
    }
}

// ── Boundary: non-Memory reload ignored by the cron loop ──────────────

/// Boundary: a non-Memory section reload must not trigger the cron loop's
/// reload handler (issue #3581). A Memory section IS cached — the trap
/// that would enable the miner if the Gateway event were wrongly
/// classified as a Memory reload — so the miner staying disabled proves
/// the event was ignored.
///
/// Conservative fixed window (#3148 item 4, mirroring
/// `test_config_change_non_memory_ignored`): a non-Memory event is
/// intentionally a no-op with no observable state change to poll; the
/// 200ms only needs to cover the spawned cron loop reaching its first
/// select poll, where the buffered event is drained.
#[tokio::test]
async fn test_cron_loop_ignores_non_memory_reload() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    seed_memory_section_cache_with_schedule(&config_manager, Some("* * * * *"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-cron-ignore.db"),
        &tmp.path().join("MEMORY-cron-ignore.md"),
    );
    let scheduler = make_config_change_scheduler(pipeline, miner.clone(), config_manager.clone())
        .with_schedule(Some("0 3 * * *".to_string()));

    notify_section_reload(&config_manager, ConfigSection::Gateway, "gateway.json");

    run_scheduler_briefly(scheduler, async {
        tokio::time::sleep(Duration::from_millis(200)).await
    })
    .await;

    assert!(
        !miner.is_enabled(),
        "non-Memory reload must not be applied by the cron loop"
    );
}

// ── Loop-level Fire persistence (issue #3608) ─────────────────────────

/// Await `n` Fires, bounded by a virtual-clock horizon of `horizon_secs`.
///
/// Under `start_paused` the timeout deadline is also virtual: auto-advance
/// drives it while the test task is parked on the recvs, so the horizon
/// discriminates on the SCHEDULING cadence, not on real runtime speed.
/// Returns the virtual time consumed by the `n` fires.
async fn await_n_fires(
    fire_rx: &mut mpsc::UnboundedReceiver<()>,
    n: usize,
    horizon_secs: u64,
) -> Duration {
    let started_at = tokio::time::Instant::now();
    let horizon = Duration::from_secs(horizon_secs);
    tokio::time::timeout(horizon, async {
        for _ in 0..n {
            fire_rx
                .recv()
                .await
                .expect("fire signal channel stays open while the loop runs");
        }
    })
    .await
    .expect("fires must keep arriving within the virtual horizon");
    started_at.elapsed()
}

/// Signal shutdown and join the loop.
async fn shutdown_and_join(shutdown_tx: watch::Sender<()>, handle: tokio::task::JoinHandle<()>) {
    shutdown_tx.send(()).expect("shutdown send");
    handle.await.expect("scheduler loop joins after shutdown");
}

/// Loop-level regression (issue #3608, normal path): after a
/// Memory-section reload switches the cron from the daily `0 3 * * *` to
/// the every-minute `* * * * *`, every subsequent Fire must re-arm on the
/// reloaded schedule instead of silently reverting to the schedule the
/// loop started with. Pre-fix, only the reload-scheduled Fire lands on
/// the new schedule and the re-arm after it jumps back to the old daily
/// 3am horizon — the second Fire cannot arrive within the horizon.
///
/// Runs on a paused (virtual) clock, same pattern as
/// `test_dreaming_shutdown_abandons_task_after_grace_period`:
/// `cron_next_fire` derives its delay from the wall clock, so pausing
/// tokio's clock lets the test fast-forward through minute boundaries.
/// Four fires (the reload-scheduled one plus three re-arms) must all
/// arrive within one virtual hour: on the every-minute schedule they
/// arrive within ~4 minutes, while a loop that reverted to the daily
/// schedule would need up to 24 virtual hours for its next fire. The
/// elapsed upper bound additionally pins the minute cadence itself
/// (four fires ≤300s): degrading to the 600s fixed-interval fallback
/// would span ≈1860s — still under the horizon, but past the bound.
#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn test_cron_loop_fires_keep_reloaded_schedule() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    seed_memory_section_cache_with_schedule(&config_manager, Some("* * * * *"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-cron-fire-persist.db"),
        &tmp.path().join("MEMORY-cron-fire-persist.md"),
    );
    let (fire_tx, mut fire_rx) = mpsc::unbounded_channel();
    let mut scheduler = make_config_change_scheduler_with_config(
        pipeline,
        miner,
        config_manager.clone(),
        MockConfig::empty().with_on_list_agents(fire_tx),
    )
    .with_schedule(Some("0 3 * * *".to_string()));

    notify_section_reload(&config_manager, ConfigSection::Memory, "memory.json");

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move { scheduler.run(shutdown_rx).await });

    let elapsed = await_n_fires(&mut fire_rx, 4, 3600).await;
    assert!(
        elapsed <= Duration::from_secs(300),
        "every-minute re-arms must keep the minute cadence: four fires \
         must span ≤300s (a 600s-fallback regression needs ≈1860s), \
         took {elapsed:?}"
    );

    shutdown_and_join(shutdown_tx, handle).await;
}

/// Loop-level regression (issue #3608, boundary): a reload to an INVALID
/// expression stores the raw value and re-arms on the fixed interval
/// (`now + dreaming_interval_secs`), and that fallback must PERSIST
/// across subsequent Fires — each Fire re-arms on the fallback again, not
/// on the stale pre-reload daily schedule. Pre-fix, the re-arm after the
/// first fallback Fire jumps back to the daily schedule and no further
/// fire can arrive within the horizon.
///
/// Paused clock; `MockConfig::dreaming_interval_secs` is 600s, so three
/// persistent-fallback fires take ~1800 virtual seconds. The horizon
/// upper-bounds the cadence (persistence, ≤1h) and the elapsed floor
/// lower-bounds it (each re-arm honours the full 600s interval: three
/// fires need ≥2×600s — a faster re-arm would mean the fallback interval
/// is not what governs the re-arms).
#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn test_cron_loop_invalid_reload_fallback_persists_across_fires() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_manager = make_test_config_manager(&tmp.path().join("config"));
    seed_memory_section_cache_with_schedule(&config_manager, Some("not-a-cron"));
    let (pipeline, miner) = make_disabled_pipeline_and_miner(
        &tmp.path().join("memory-cron-fallback-persist.db"),
        &tmp.path().join("MEMORY-cron-fallback-persist.md"),
    );
    let (fire_tx, mut fire_rx) = mpsc::unbounded_channel();
    let mut scheduler = make_config_change_scheduler_with_config(
        pipeline,
        miner,
        config_manager.clone(),
        MockConfig::empty().with_on_list_agents(fire_tx),
    )
    .with_schedule(Some("0 3 * * *".to_string()));

    notify_section_reload(&config_manager, ConfigSection::Memory, "memory.json");

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async move { scheduler.run(shutdown_rx).await });

    let elapsed = await_n_fires(&mut fire_rx, 3, 3600).await;
    assert!(
        elapsed >= Duration::from_secs(1200),
        "each re-arm must honour the full 600s fallback interval: three \
         fires must span ≥1200s, took {elapsed:?}"
    );

    shutdown_and_join(shutdown_tx, handle).await;
}
