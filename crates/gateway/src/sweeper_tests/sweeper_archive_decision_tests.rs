//! Archive determination tests: pending operations never block archiving,
//! and any active SessionActivityDimensions dimension skips it.

use closeclaw_common::SessionActivityDimensions;
use closeclaw_session::persistence::{PendingOperationDetail, SessionCheckpoint};
use std::sync::Arc;

use super::sweeper_test_utils::{sweeper_with_agents, MockActiveQueryWithDimensions};

// ── archive determination behavior tests ─────────────────────────────────

/// Pending operations non-empty + all four dimensions false + idle →
/// still archived (archive does NOT depend on pending_operations).
#[tokio::test]
async fn test_pending_operations_non_empty_still_archives_when_all_dimensions_false() {
    use chrono::Utc;
    use closeclaw_session::persistence::{
        PendingOperation, PendingOperationStatus, PendingOperationType,
    };

    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("pend-but-archive".into());

    let mut cp = SessionCheckpoint::new("pend-but-archive".into());
    cp = cp.with_pending_operations(vec![PendingOperation {
        op_id: "op-1".into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::ToolCall {
            tool_name: "bash".into(),
            args_summary: "{}".into(),
        },
        created_at: Utc::now(),
    }]);
    mem.add_checkpoint(cp);

    // active_query returns all-false (session not actively executing)
    let sweeper = sweeper.with_active_query(Arc::new(MockActiveQueryWithDimensions::new(vec![])));
    sweeper.run_once().await.unwrap();

    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        archive_called.contains(&"pend-but-archive".into()),
        "pending_operations non-empty but all dimensions false → must still archive; \
         actual archive calls: {archive_called:?}"
    );
}

/// Any single active dimension → skip archive.
/// (Four sub-cases: llm, foreground, background, child.)
#[tokio::test]
async fn test_any_single_active_dimension_skips_archive() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("dim-llm".into());
    mem.add_idle_session("dim-fg".into());
    mem.add_idle_session("dim-bg".into());
    mem.add_idle_session("dim-child".into());
    for id in ["dim-llm", "dim-fg", "dim-bg", "dim-child"] {
        mem.add_checkpoint(SessionCheckpoint::new(id.into()));
    }

    // Each session has exactly one dimension true.
    let sweeper = sweeper.with_active_query(Arc::new(MockActiveQueryWithDimensions::new(vec![
        (
            "dim-llm".into(),
            SessionActivityDimensions {
                llm_active: true,
                ..Default::default()
            },
        ),
        (
            "dim-fg".into(),
            SessionActivityDimensions {
                foreground_tool_active: true,
                ..Default::default()
            },
        ),
        (
            "dim-bg".into(),
            SessionActivityDimensions {
                background_tool_active: true,
                ..Default::default()
            },
        ),
        (
            "dim-child".into(),
            SessionActivityDimensions {
                child_active: true,
                ..Default::default()
            },
        ),
    ])));
    sweeper.run_once().await.unwrap();

    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        !archive_called.contains(&"dim-llm".into()),
        "llm_active=true → must skip archive; actual archive calls: {archive_called:?}"
    );
    assert!(
        !archive_called.contains(&"dim-fg".into()),
        "foreground_tool_active=true → must skip archive; actual archive calls: {archive_called:?}"
    );
    assert!(
        !archive_called.contains(&"dim-bg".into()),
        "background_tool_active=true → must skip archive; actual archive calls: {archive_called:?}"
    );
    assert!(
        !archive_called.contains(&"dim-child".into()),
        "child_active=true → must skip archive; actual archive calls: {archive_called:?}"
    );
}

/// Session not in memory (ActiveSessionQuery returns all false) → normal archive.
/// (Document: not in memory = no active operations.)
#[tokio::test]
async fn test_session_not_in_memory_archives_normally() {
    let (mem, sweeper) = sweeper_with_agents(vec!["agent-x".into()]);
    mem.add_idle_session("not-in-mem".into());
    mem.add_checkpoint(SessionCheckpoint::new("not-in-mem".into()));

    let sweeper = sweeper.with_active_query(Arc::new(MockActiveQueryWithDimensions::new(vec![])));
    sweeper.run_once().await.unwrap();

    let archive_called = mem.archive_called.lock().unwrap();
    assert!(
        archive_called.contains(&"not-in-mem".into()),
        "session not in memory (all dimensions false) → must archive; \
         actual archive calls: {archive_called:?}"
    );
}
