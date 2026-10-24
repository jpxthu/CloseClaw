//! Lifecycle test helpers: the shared Phase 3 stop budget anchor and the
//! shutdown-heartbeat observation loop skeleton.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::lifecycle::{BG_TASK_ABORT_GRACE_SECS, BG_TASK_JOIN_TIMEOUT_SECS};
use crate::shutdown_heartbeat::ShutdownHeartbeat;

// ── Shared Phase 3 stop budget (test-side anchor) ────────────────────────

/// Total Phase 3 stop budget for one background task: 10s
/// (= join 7s + abort grace 3s — compile-time asserted by
/// `lifecycle::bg_task_helpers::wait_all_bg_tasks`). Anchored on the
/// production second constants so test assertions track production
/// values at compile time. Single definition shared by the lifecycle
/// test files: join-wait tests use it as their upper bound and
/// grace-window tests as the grace period, mirroring
/// `phase_3_background_stop`.
pub(crate) const PHASE3_STOP_TOTAL_BUDGET: Duration =
    Duration::from_secs(BG_TASK_JOIN_TIMEOUT_SECS + BG_TASK_ABORT_GRACE_SECS);

// ── Shared heartbeat observation helper ──────────────────────────────────

/// Observe a [`ShutdownHeartbeat`] over a bounded window — the shared
/// skeleton of the shutdown-phase heartbeat observation loops (issue
/// #3718): each iteration selects between the heartbeat deadline
/// (`sleep_until(next_deadline())` + `should_send_heartbeat()` gate +
/// count + `record_event()`) and the test's event stream, until
/// `deadline` passes or the stream says stop.
///
/// - `events`: the loop's second select branch, abstracted as an event
///   stream. Producers are `tokio::spawn`ed and send over the channel —
///   a one-shot completion sends once and exits, a repeated signal source
///   keeps sending while the caller holds the sender alive; a dropped
///   sender ends the stream.
/// - `on_event`: called for every event **after** the heartbeat timer is
///   reset (an event is progress — the helper owns `record_event()`
///   because `heartbeat` is already mutably borrowed here). Return `true`
///   to break out of the window early (e.g. the awaited task completed).
/// - `on_heartbeat`: called after each heartbeat is counted (the
///   production counterpart sends the heartbeat card).
///
/// Returns the number of heartbeats sent inside the window; callers keep
/// their own assertions on that count.
pub(crate) async fn observe_heartbeat_until<T>(
    heartbeat: &mut ShutdownHeartbeat,
    deadline: Instant,
    mut events: mpsc::UnboundedReceiver<T>,
    mut on_event: impl FnMut(T) -> bool,
    mut on_heartbeat: impl FnMut(),
) -> usize {
    let mut sent = 0usize;
    while Instant::now() < deadline {
        tokio::select! {
            _ = tokio::time::sleep_until(heartbeat.next_deadline()) => {
                if heartbeat.should_send_heartbeat() {
                    sent += 1;
                    heartbeat.record_event();
                    on_heartbeat();
                }
            }
            event = events.recv() => {
                let Some(event) = event else {
                    break; // producer dropped: stream ended, stop observing
                };
                heartbeat.record_event();
                if on_event(event) {
                    break;
                }
            }
        }
    }
    sent
}
