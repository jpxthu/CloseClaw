//! ArchiveSweeper::run_once tests: single sweep pass semantics.

use closeclaw_config::session::PerAgentSessionConfig;
use closeclaw_session::persistence::SessionCheckpoint;

use super::sweeper_test_utils::{sweeper_with_agents, sweeper_with_session_config, StorageFault};

// -----------------------------------------------------------------
// Test: run_once calls archive for idle sessions
// -----------------------------------------------------------------

#[tokio::test]
async fn test_run_once_calls_archive() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("session-1".into());
    mem.add_checkpoint(SessionCheckpoint::new("session-1".into()));

    sweeper.run_once().await.unwrap();

    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        archive_called.contains(&"session-1".into()),
        "run_once must archive idle session-1; actual archive calls: {archive_called:?}"
    );
}

// -----------------------------------------------------------------
// Test: run_once calls purge for expired archived sessions
// -----------------------------------------------------------------

#[tokio::test]
async fn test_run_once_calls_purge() {
    let (mem, sweeper) = sweeper_with_session_config(
        vec!["agent-x".into()],
        // Default purge_after_minutes is 0 (never purge); set to non-zero
        // so the purge path is exercised.
        PerAgentSessionConfig::new(30, 60, false),
    );
    mem.add_expired_session("session-2".into());

    sweeper.run_once().await.unwrap();

    let purge_called = mem.purge_called.lock().unwrap();
    assert!(
        purge_called.contains(&"session-2".into()),
        "run_once must purge expired session-2; actual purge calls: {purge_called:?}"
    );
}

// -----------------------------------------------------------------
// Test: purge_after_minutes = 0 skips purge scan
// -----------------------------------------------------------------

#[tokio::test]
async fn test_purge_after_zero_skips_purge() {
    let (mem, sweeper) = sweeper_with_session_config(
        vec!["agent-x".into()],
        PerAgentSessionConfig::new(30, 0, false),
    );
    mem.add_expired_session("session-x".into());

    let result = sweeper.run_once().await;

    assert!(
        result.is_ok(),
        "run_once must return Ok when purge is disabled, actual: {result:?}"
    );
    let purge_called = mem.purge_called.lock().unwrap();
    assert!(
        purge_called.is_empty(),
        "purge_after_minutes=0 must skip purge scan; actual purge calls: {purge_called:?}"
    );
}

// -----------------------------------------------------------------
// Test: no agents returns Ok without error
// -----------------------------------------------------------------

#[tokio::test]
async fn test_no_agents_no_error() {
    let (_mem, sweeper) = sweeper_with_agents(vec![]);

    let result = sweeper.run_once().await;

    assert!(
        result.is_ok(),
        "run_once must return Ok with no agents, actual: {result:?}"
    );
}

// -----------------------------------------------------------------
// Test: injected storage Err is swallowed, sweep loop keeps going
// -----------------------------------------------------------------

#[tokio::test]
async fn test_run_once_storage_err_swallowed_loop_continues() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("session-1".into());
    mem.add_checkpoint(SessionCheckpoint::new("session-1".into()));
    // Fail both role iterations (MainAgent + SubAgent) of agent-x.
    mem.inject_list_idle_fault(StorageFault::Err, 2);

    let result = sweeper.run_once().await;
    assert!(
        result.is_ok(),
        "run_once must swallow the injected storage Err and return Ok, actual: {result:?}"
    );
    assert_eq!(
        mem.list_idle_calls(),
        2,
        "sweep must continue to the next role after a storage Err (2 list_idle calls expected)"
    );
    {
        let archive_called = mem.archive_called.lock().unwrap();
        assert!(
            archive_called.is_empty(),
            "no session must be archived while list_idle errors; \
             actual archive calls: {archive_called:?}"
        );
    }

    // Fault budget exhausted: the following run_once must still run both
    // role iterations and archive — the earlier errors must neither stop
    // the loop nor pollute state.
    let retry = sweeper.run_once().await;
    assert!(
        retry.is_ok(),
        "run_once after the injected errors must return Ok, actual: {retry:?}"
    );
    assert_eq!(
        mem.list_idle_calls(),
        4,
        "recovered run_once must sweep both roles again (4 list_idle calls total)"
    );
    {
        let archive_called = mem.archive_called.lock().unwrap();
        assert!(
            archive_called.contains(&"session-1".into()),
            "recovered run_once must archive session-1; actual archive calls: {archive_called:?}"
        );
    }

    // Fault-injection contract: `count == 0` disarms instead of arming
    // one fault, so this run must sweep both roles healthy and archive
    // twice (once per role) — a spurious fault would archive only once
    // (the MainAgent iteration would error out).
    mem.inject_list_idle_fault(StorageFault::Err, 0);
    let archive_before = mem.archive_called.lock().unwrap().len();
    let disarmed = sweeper.run_once().await;
    assert!(
        disarmed.is_ok(),
        "run_once with a disarmed fault (count == 0) must return Ok, actual: {disarmed:?}"
    );
    let archive_delta = mem.archive_called.lock().unwrap().len() - archive_before;
    assert_eq!(
        archive_delta, 2,
        "count == 0 must not fire a fault: both roles must archive session-1 \
         (expected archive delta 2, actual {archive_delta})"
    );
}

// -----------------------------------------------------------------
// Test: run_once catches storage panic (loop not stopped)
// -----------------------------------------------------------------

#[tokio::test]
async fn test_run_once_storage_panic_is_caught() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("session-1".into());
    mem.add_checkpoint(SessionCheckpoint::new("session-1".into()));
    mem.inject_list_idle_fault(StorageFault::Panic, 1);

    let result = sweeper.run_once().await;
    assert!(
        result.is_ok(),
        "run_once must catch the injected storage panic (catch_unwind) \
         and return Ok, actual: {result:?}"
    );
    assert_eq!(
        mem.list_idle_calls(),
        1,
        "the injected panic must actually fire on the first list_idle call and abort this sweep"
    );
    {
        let archive_called = mem.archive_called.lock().unwrap();
        assert!(
            archive_called.is_empty(),
            "panicked sweep must not archive anything; actual archive calls: {archive_called:?}"
        );
    }

    // Fault budget exhausted: repeated run_once must keep working and
    // archive normally — a caught panic must not stop the loop or
    // pollute state.
    let retry = sweeper.run_once().await;
    assert!(
        retry.is_ok(),
        "run_once after the caught panic must return Ok, actual: {retry:?}"
    );
    assert_eq!(
        mem.list_idle_calls(),
        3,
        "post-panic run_once must sweep both roles \
         (3 list_idle calls total: 1 panicked + 2 healthy)"
    );
    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        archive_called.contains(&"session-1".into()),
        "post-panic run_once must archive session-1; actual archive calls: {archive_called:?}"
    );
}
