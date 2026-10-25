//! Shutdown test helpers: the gateway `ShutdownHandle` bridge, self
//! signal delivery, and the shared signal-step timeout wrapper.

use std::sync::Arc;
use std::time::Duration;

/// Duplicate of `crate::bridge::common_shutdown_handle` for daemon-crate tests.
/// Creates a `closeclaw_gateway::shutdown_handle::ShutdownHandle` from the daemon's
/// `ShutdownHandle`.
pub fn common_shutdown_handle(
    daemon_handle: &crate::shutdown::ShutdownHandle,
) -> Arc<closeclaw_gateway::shutdown_handle::ShutdownHandle> {
    Arc::new(closeclaw_gateway::shutdown_handle::ShutdownHandle::new(
        Arc::new(daemon_handle.clone()),
    ))
}

/// Graceful-shutdown signals a test may deliver to its own process.
///
/// Test-only type carrying the SIGTERM/SIGINT-only invariant of
/// [`kill_self`] in code instead of a caller convention: every variant is
/// one of the two graceful first-signal shutdowns defined by
/// `docs/design/daemon/shutdown.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestShutdownSignal {
    /// SIGTERM — graceful shutdown trigger.
    Sigterm,
    /// SIGINT — graceful shutdown trigger.
    Sigint,
}

impl TestShutdownSignal {
    /// The libc signal number this variant delivers.
    pub(crate) fn as_libc_signal(self) -> libc::c_int {
        match self {
            TestShutdownSignal::Sigterm => libc::SIGTERM,
            TestShutdownSignal::Sigint => libc::SIGINT,
        }
    }
}

/// Send shutdown signal `sig` to the current process itself.
///
/// Single point wrapping the libc kill call for daemon tests — the signal
/// is chosen through [`TestShutdownSignal`], so the type, not caller
/// convention, limits delivery to the graceful SIGTERM/SIGINT pair.
pub(crate) fn kill_self(sig: TestShutdownSignal) {
    // SAFETY: the target pid is `std::process::id()`, i.e. this process
    // itself, so the signal is delivered only to the calling process and
    // never to another one; `sig` is a `TestShutdownSignal`, whose only
    // variants are the graceful SIGTERM/SIGINT shutdown signals, so the
    // delivered signal's effect on this process is exactly what the
    // surrounding test exercises.
    //
    // The return value is checked so a failed kill surfaces the OS error
    // immediately instead of letting the test hang on the un-sent signal.
    let ret = unsafe { libc::kill(std::process::id() as libc::pid_t, sig.as_libc_signal()) };
    assert_eq!(
        ret,
        0,
        "kill self failed: {}",
        std::io::Error::last_os_error()
    );
}

// ── Shared signal-step timeout helper ────────────────────────────────────

/// Awaits one signal-delivery step under a 10s timeout, panicking
/// with `signal not delivered: {what}` when it does not complete (STANDARDS
/// §6). Call sites keep unwrapping the future's own output (STANDARDS §9).
///
/// Extracted verbatim from `shutdown_alignment_tests.rs` (issue #3697): one
/// crate-shared definition instead of a file-private copy per shutdown test
/// file.
pub async fn signal_step<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), fut)
        .await
        .unwrap_or_else(|_| panic!("signal not delivered: {what}"))
}

// ── signal_step helper behavior ──────────────────────────────────────

/// Normal path: a completing future's output passes through unchanged —
/// the helper is a transparent 10s timeout wrapper.
#[tokio::test]
async fn test_signal_step_passes_output_through() {
    let value = signal_step("passthrough completes in time", async { 42 }).await;
    assert_eq!(
        value, 42,
        "signal_step must return the wrapped future's output"
    );
}

/// Timeout path: a never-completing step must panic with the
/// `signal not delivered: {what}` diagnostic instead of hanging until the
/// 30s harness limit with no diagnosis (STANDARDS §6). Paused clock:
/// auto-advance fires the virtual 10s deadline, so no real 10s wait.
#[tokio::test(start_paused = true)]
async fn test_signal_step_panics_with_diagnostic_on_timeout() {
    let joined = tokio::spawn(async {
        signal_step("hung step diagnostic", std::future::pending::<()>()).await
    })
    .await;
    let join_err =
        joined.expect_err("signal_step must panic when the wrapped step never completes");
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
        msg.contains("signal not delivered: hung step diagnostic"),
        "expected the signal not delivered diagnostic carrying the what \
         label, got: {msg}"
    );
}
