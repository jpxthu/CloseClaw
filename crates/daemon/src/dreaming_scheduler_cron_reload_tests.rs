//! Unit tests for the DreamingScheduler cron-loop config-reload path
//! (issue #3581): `handle_cron_config_reload` — schedule switching,
//! next-fire recomputation from the reloaded schedule, invalid-schedule
//! fallback to the fixed interval, and non-Memory reloads being ignored
//! by the cron loop.
//!
//! Own file (not appended to `dreaming_scheduler_tests.rs`) because that
//! file sits near the 1000-line hard limit (CONTRIBUTING §硬性限制).

use std::sync::Arc;

use crate::test_helpers::{
    make_config_change_scheduler, make_disabled_pipeline_and_miner, make_test_config_manager,
    notify_section_reload, run_scheduler_briefly, seed_memory_section_cache_with_schedule,
    wait_until,
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
    let mut next_fire = before + tokio::time::Duration::from_secs(600);

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
        next_fire <= before + tokio::time::Duration::from_secs(60),
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
        let fixed_lo = before + tokio::time::Duration::from_secs(600);
        let fixed_hi = after + tokio::time::Duration::from_secs(600);
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
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await
    })
    .await;

    assert!(
        !miner.is_enabled(),
        "non-Memory reload must not be applied by the cron loop"
    );
}
