//! Reusable heartbeat component for shutdown phases.
//!
//! During graceful shutdown, when no state changes occur for a while,
//! the daemon sends periodic heartbeat notifications to the Owner
//! confirming the system is still shutting down (not stuck).
//!
//! Design reference: `docs/design/daemon/shutdown.md` — "Owner 进度通知"
//! section: "心跳在存在等待的停止阶段生效（Phase 1 drain 等待、Phase 2
//! Session 停止、Phase 3 后台任务停止）：期间无状态变化时每 30 秒发送一次；

use std::time::Duration;

/// Default heartbeat interval: 30 seconds.
pub(crate) const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Reusable heartbeat timer for shutdown phases.
///
/// Tracks the last event time and provides a method to check whether
/// a heartbeat should be sent. When `should_send_heartbeat()` returns
/// `true`, the caller sends the heartbeat card and calls `record_event()`
/// to reset the timer.
///
/// # Usage in `tokio::select!`
///
/// ```ignore
/// loop {
///     tokio::select! {
///         result = &mut task => { /* task done */ break; }
///         _ = sigint.recv() => { /* escalation */ }
///         _ = tokio::time::sleep_until(heartbeat.next_deadline()) => {
///             if heartbeat.should_send_heartbeat() {
///                 gateway.send_shutdown_heartbeat_card(...).await;
///                 heartbeat.record_event();
///             }
///         }
///     }
///     heartbeat.record_event(); // on any progress event
/// }
/// ```
pub(crate) struct ShutdownHeartbeat {
    /// Interval between heartbeats.
    interval: Duration,
    /// Timestamp of the last event or heartbeat send.
    last_event: tokio::time::Instant,
    /// When this heartbeat tracker was created (phase start).
    phase_start: tokio::time::Instant,
}

impl ShutdownHeartbeat {
    /// Create a new heartbeat tracker with the default 30s interval.
    pub(crate) fn new() -> Self {
        Self::with_interval(DEFAULT_HEARTBEAT_INTERVAL)
    }

    /// Create a new heartbeat tracker with a custom interval.
    ///
    /// Useful for testing with short intervals.
    pub(crate) fn with_interval(interval: Duration) -> Self {
        Self {
            interval,
            last_event: tokio::time::Instant::now(),
            phase_start: tokio::time::Instant::now(),
        }
    }

    /// Create a heartbeat tracker for testing with a custom start time.
    #[cfg(test)]
    pub(crate) fn with_start(start: tokio::time::Instant, interval: Duration) -> Self {
        Self {
            interval,
            last_event: start,
            phase_start: start,
        }
    }

    /// Returns the deadline for the next heartbeat sleep.
    ///
    /// Use with `tokio::time::sleep_until()` in `tokio::select!`.
    /// The deadline is `last_event + interval`. If an event arrives
    /// before the deadline, the select branch is not taken (the future
    /// is still pending). After the branch fires, call
    /// `should_send_heartbeat()` to confirm, then `record_event()`.
    pub(crate) fn next_deadline(&self) -> tokio::time::Instant {
        self.last_event + self.interval
    }

    /// Returns `true` if a heartbeat should be sent.
    ///
    /// This checks that at least `interval` has elapsed since the last
    /// event. Always call this after the sleep branch fires in
    /// `tokio::select!` to confirm the heartbeat is still warranted
    /// (an event may have arrived in the same poll cycle).
    pub(crate) fn should_send_heartbeat(&self) -> bool {
        self.last_event.elapsed() >= self.interval
    }

    /// Record that an event occurred (progress, signal, task completion).
    ///
    /// Resets the heartbeat timer so the next heartbeat won't fire
    /// until `interval` after this event.
    pub(crate) fn record_event(&mut self) {
        self.last_event = tokio::time::Instant::now();
    }

    /// Returns the elapsed time since the phase started, in seconds.
    ///
    /// Used as the `longest_wait_secs` parameter for
    /// `send_shutdown_heartbeat_card`.
    pub(crate) fn elapsed_secs(&self) -> u64 {
        self.phase_start.elapsed().as_secs()
    }

    /// Returns the configured interval.
    #[cfg(test)]
    pub(crate) fn interval(&self) -> Duration {
        self.interval
    }
}

impl Default for ShutdownHeartbeat {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_heartbeat_default_interval() {
        let hb = ShutdownHeartbeat::new();
        assert_eq!(hb.interval(), DEFAULT_HEARTBEAT_INTERVAL);
    }

    #[test]
    fn test_heartbeat_custom_interval() {
        let hb = ShutdownHeartbeat::with_interval(Duration::from_secs(5));
        assert_eq!(hb.interval(), Duration::from_secs(5));
    }

    #[test]
    fn test_should_send_heartbeat_after_interval() {
        let start = tokio::time::Instant::now();
        let hb = ShutdownHeartbeat::with_start(start, Duration::from_millis(10));
        // Immediately: should NOT send
        assert!(!hb.should_send_heartbeat());
        // After interval: should send
        let hb2 = ShutdownHeartbeat::with_start(
            start - Duration::from_millis(20),
            Duration::from_millis(10),
        );
        assert!(hb2.should_send_heartbeat());
    }

    #[test]
    fn test_record_event_resets_timer() {
        let start = tokio::time::Instant::now();
        let mut hb = ShutdownHeartbeat::with_start(start, Duration::from_millis(10));
        // Simulate time passing
        let hb_old = ShutdownHeartbeat::with_start(
            start - Duration::from_millis(20),
            Duration::from_millis(10),
        );
        assert!(hb_old.should_send_heartbeat());

        // Record event resets the timer
        hb.record_event();
        assert!(!hb.should_send_heartbeat());
    }

    #[test]
    fn test_next_deadline_is_last_event_plus_interval() {
        let start = tokio::time::Instant::now();
        let hb = ShutdownHeartbeat::with_start(start, Duration::from_secs(30));
        let deadline = hb.next_deadline();
        // Deadline should be ~30s from start
        let diff = deadline.duration_since(start);
        assert!(
            diff >= Duration::from_secs(29) && diff <= Duration::from_secs(31),
            "deadline should be ~30s from start, got {:?}",
            diff
        );
    }

    #[test]
    fn test_elapsed_secs_reflects_phase_start() {
        let start = tokio::time::Instant::now() - Duration::from_secs(5);
        let hb = ShutdownHeartbeat::with_start(start, Duration::from_secs(30));
        let elapsed = hb.elapsed_secs();
        assert!(
            (4..=6).contains(&elapsed),
            "elapsed should be ~5s, got {}",
            elapsed
        );
    }

    #[test]
    fn test_default_impl_matches_new() {
        let hb = ShutdownHeartbeat::default();
        assert_eq!(hb.interval(), DEFAULT_HEARTBEAT_INTERVAL);
    }
}
