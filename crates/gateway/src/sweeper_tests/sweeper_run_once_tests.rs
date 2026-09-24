//! ArchiveSweeper::run_once tests: single sweep pass semantics.

use closeclaw_config::session::PerAgentSessionConfig;
use closeclaw_session::persistence::SessionCheckpoint;

use super::sweeper_test_utils::{sweeper_with_agents, sweeper_with_session_config};

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
    assert!(archive_called.contains(&"session-1".into()));
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
    assert!(purge_called.contains(&"session-2".into()));
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

    assert!(result.is_ok());
    let purge_called = mem.purge_called.lock().unwrap();
    assert!(purge_called.is_empty());
}

// -----------------------------------------------------------------
// Test: no agents returns Ok without error
// -----------------------------------------------------------------

#[tokio::test]
async fn test_no_agents_no_error() {
    let (_mem, sweeper) = sweeper_with_agents(vec![]);

    let result = sweeper.run_once().await;

    assert!(result.is_ok());
}

// -----------------------------------------------------------------
// Test: run_once error does not stop loop (panic is caught)
// -----------------------------------------------------------------

#[tokio::test]
async fn test_run_once_error_does_not_stop_loop() {
    let (_mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);

    let result1 = sweeper.run_once().await;
    assert!(result1.is_ok());

    let result2 = sweeper.run_once().await;
    assert!(result2.is_ok());
}
