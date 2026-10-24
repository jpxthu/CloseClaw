//! Unit tests for the `emit_event` early-return branches against a
//! real `DebugLog` write path.
//!
//! The wrapper-level no-op tests (inline in `emit.rs` and expanded by
//! `test_macros.rs`) only verify "does not panic". These tests strengthen
//! no-op semantics to "zero output": with a real `DebugLog` writing to a
//! `TempDir`, an early return must leave the JSONL event files empty.
//!
//! Event reading/polling helpers come from `crate::test_utils` (gated
//! `any(test, feature = "test-util")`, reachable under `cfg(test)`
//! without the feature) so the standard acceptance command
//! `cargo nextest run -p closeclaw-debug-log` covers them.

use std::time::Duration;

use tempfile::TempDir;

use super::{emit_event, DebugLogContext, EmitEventParams};
use crate::test_utils::{make_debug_log_at_level, poll_event_count};
use crate::LogLevel;

/// Deadline for the zero-output absence polls (repo precedent: the
/// gateway `None`-identity zero-emission assertion uses 500ms).
const ABSENCE_POLL_DEADLINE: Duration = Duration::from_millis(500);
/// Deadline for the positive-path control assertion (failure path only —
/// the write lands in milliseconds, so the happy path exits early).
const POSITIVE_POLL_DEADLINE: Duration = Duration::from_secs(5);

fn emit_params(ctx: DebugLogContext<'_>) -> EmitEventParams<'_> {
    EmitEventParams {
        ctx,
        level: LogLevel::Info,
        source_module: "test",
        event_type: "test.event",
        payload: serde_json::json!({}),
        parent: None,
    }
}

/// Empty trace_id must early-return before the write stage: with a real
/// `DebugLog` wired into the context, no JSONL file or event may appear
/// within the absence-poll window (an erroneously spawned write would
/// land in milliseconds and be caught).
#[tokio::test]
async fn test_empty_trace_id_with_real_debug_log_writes_zero_events() {
    let tmp = TempDir::new().expect("TempDir::new failed");
    let debug_log = make_debug_log_at_level(&tmp, LogLevel::Debug).await;
    let ctx = DebugLogContext::new(Some(&debug_log), "", None);
    emit_event(emit_params(ctx));

    let count = poll_event_count(tmp.path(), ABSENCE_POLL_DEADLINE).await;
    assert_eq!(
        count, 0,
        "empty trace_id must early-return without writing any event"
    );
}

/// `None` debug_log must early-return before the write stage even though
/// a real write path exists on disk: no JSONL file or event may appear
/// within the absence-poll window.
#[tokio::test]
async fn test_none_debug_log_with_real_write_path_writes_zero_events() {
    let tmp = TempDir::new().expect("TempDir::new failed");
    let _real_debug_log = make_debug_log_at_level(&tmp, LogLevel::Debug).await;
    let ctx = DebugLogContext::new(None, "trace-none", None);
    emit_event(emit_params(ctx));

    let count = poll_event_count(tmp.path(), ABSENCE_POLL_DEADLINE).await;
    assert_eq!(
        count, 0,
        "None debug_log must early-return without writing any event"
    );
}

/// Positive control proving the write path itself is live: the same emit
/// with a non-empty trace_id and `Some` debug_log must land exactly one
/// event (deadline-bounded polling; the write happens in a spawned task).
#[tokio::test]
async fn test_non_empty_trace_id_emits_and_writes_one_event() {
    let tmp = TempDir::new().expect("TempDir::new failed");
    let debug_log = make_debug_log_at_level(&tmp, LogLevel::Debug).await;
    let ctx = DebugLogContext::new(Some(&debug_log), "trace-live", None);
    emit_event(emit_params(ctx));

    let count = poll_event_count(tmp.path(), POSITIVE_POLL_DEADLINE).await;
    assert_eq!(
        count, 1,
        "non-empty trace_id with Some debug_log must write exactly one event"
    );
}
