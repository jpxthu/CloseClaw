// Shared test-support code for the `process_tests` topic modules
// (issue #4050): child-process spawning plus the self-signal wrapper.
// Declared from `mod.rs` as `#[cfg(test)] mod support;` so the mount
// resolution of `scripts/test-audit.sh` keeps covering this file.

use crate::process::{subscribe_shutdown_signals, ShutdownSignalSubscription};

/// Helper: spawn a long-running child process and return it.
pub(crate) fn spawn_sleep_child() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("60")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn sleep child")
}

// Signal tests use subprocess isolation to prevent cross-contamination.
// Parent tests spawn the child in a subprocess; child tests are gated
// by `SIGNAL_TEST_CHILD=1` to avoid running in the parent process.

/// Graceful-shutdown signals a test may deliver to its own process.
///
/// Test-only type carrying the SIGTERM/SIGINT-only invariant of
/// [`kill_self`] in code instead of a caller convention: every variant
/// is one of the two graceful shutdown signals handled by
/// `crate::process::subscribe_shutdown_signals`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestSelfSignal {
    /// SIGTERM — graceful shutdown trigger.
    Sigterm,
    /// SIGINT — graceful shutdown trigger.
    Sigint,
}

impl TestSelfSignal {
    /// The libc signal number this variant delivers.
    pub(crate) fn as_libc_signal(self) -> libc::c_int {
        match self {
            TestSelfSignal::Sigterm => libc::SIGTERM,
            TestSelfSignal::Sigint => libc::SIGINT,
        }
    }

    /// The OS name of this signal (e.g. "SIGTERM"), for diagnostics.
    pub(crate) fn name(self) -> &'static str {
        match self {
            TestSelfSignal::Sigterm => "SIGTERM",
            TestSelfSignal::Sigint => "SIGINT",
        }
    }
}

/// Send shutdown signal `sig` to the current process itself.
///
/// Single point wrapping the libc kill call for the self-signal
/// tests — the signal is chosen through [`TestSelfSignal`], so the type,
/// not caller convention, limits delivery to the SIGTERM/SIGINT pair.
pub(crate) fn kill_self(sig: TestSelfSignal) {
    // SAFETY: the target pid is `std::process::id()`, i.e. this process
    // itself, so the signal is delivered only to the calling process and
    // never to another one; `sig` is a `TestSelfSignal`, whose only
    // variants are the SIGTERM/SIGINT shutdown signals, so the delivered
    // signal's effect on this process is exactly what the surrounding
    // test exercises.
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

/// Whether this process was spawned as a signal-test child subprocess:
/// parent tests set `SIGNAL_TEST_CHILD=1` for their children, so child
/// test bodies skip themselves when the variable is absent.
pub(crate) fn in_signal_test_child() -> bool {
    std::env::var("SIGNAL_TEST_CHILD").is_ok()
}

/// Open the shutdown-signal subscription for a child test body, panicking
/// with the shared message on failure.
pub(crate) async fn open_child_subscription() -> ShutdownSignalSubscription {
    subscribe_shutdown_signals()
        .await
        .expect("subscribe_shutdown_signals should succeed")
}
