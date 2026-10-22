//! ActiveSessionQuery integration tests for ArchiveSweeper.
//!
//! Verifies that the sweeper correctly skips archiving sessions that are
//! actively executing work (LLM call, tool execution, etc.).

use super::sweeper_test_utils::{
    sweeper_with_agents, MockActiveQuery, MockActiveQueryWithDimensions,
};
use closeclaw_session::persistence::SessionCheckpoint;
use std::sync::Arc;

// ── Tests ────────────────────────────────────────────────────────

/// With active_query injected, session marked active is NOT archived.
#[tokio::test]
async fn test_sweeper_skips_active_session() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("active-sess".into());
    mem.add_checkpoint(SessionCheckpoint::new("active-sess".into()));

    let sweeper =
        sweeper.with_active_query(Arc::new(MockActiveQueryWithDimensions::all_active(vec![
            "active-sess".into(),
        ])));
    sweeper.run_once().await.unwrap();

    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        !archive_called.contains(&"active-sess".into()),
        "session reported as active should NOT be archived; \
         actual archive calls: {archive_called:?}"
    );
}

/// Without active_query (None), inactive session IS archived (baseline).
#[tokio::test]
async fn test_sweeper_archives_inactive_session_no_active_query() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("inactive-sess".into());
    mem.add_checkpoint(SessionCheckpoint::new("inactive-sess".into()));

    sweeper.run_once().await.unwrap();

    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        archive_called.contains(&"inactive-sess".into()),
        "session with no active_query should be archived; \
         actual archive calls: {archive_called:?}"
    );
}

/// With active_query injected, session NOT in active list IS archived.
#[tokio::test]
async fn test_sweeper_archives_inactive_session_with_active_query() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("not-active-sess".into());
    mem.add_checkpoint(SessionCheckpoint::new("not-active-sess".into()));

    let sweeper = sweeper.with_active_query(Arc::new(MockActiveQuery::none()));
    sweeper.run_once().await.unwrap();

    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        archive_called.contains(&"not-active-sess".into()),
        "session not in active list should be archived; actual archive calls: {archive_called:?}"
    );
}

/// Multiple idle sessions: active one skipped, inactive one archived.
#[tokio::test]
async fn test_sweeper_mixed_active_and_inactive_sessions() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("mixed-active".into());
    mem.add_idle_session("mixed-inactive".into());
    mem.add_checkpoint(SessionCheckpoint::new("mixed-active".into()));
    mem.add_checkpoint(SessionCheckpoint::new("mixed-inactive".into()));

    let sweeper =
        sweeper.with_active_query(Arc::new(MockActiveQueryWithDimensions::all_active(vec![
            "mixed-active".into(),
        ])));
    sweeper.run_once().await.unwrap();

    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        !archive_called.contains(&"mixed-active".into()),
        "active session should be skipped; actual archive calls: {archive_called:?}"
    );
    assert!(
        archive_called.contains(&"mixed-inactive".into()),
        "inactive session should be archived; actual archive calls: {archive_called:?}"
    );
}
