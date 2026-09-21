//! Unit tests for the narrowed `kill_self` interface (issue #3140):
//! [`TestShutdownSignal`] carries the SIGTERM/SIGINT-only invariant in
//! code, so these tests pin both the type-level variant → libc encoding
//! and the end-to-end delivery each variant promises to its callers.

use crate::test_helpers::{kill_self, TestShutdownSignal};
use std::time::Duration;
use tokio::signal::unix::{signal, SignalKind};

/// Locks the variant → libc signal-number encoding: `Sigterm` must be
/// `libc::SIGTERM`, `Sigint` must be `libc::SIGINT`. A swapped mapping
/// would silently change which signal every shutdown test delivers.
#[test]
fn test_shutdown_signal_variant_maps_to_libc_signal() {
    assert_eq!(
        TestShutdownSignal::Sigterm.as_libc_signal(),
        libc::SIGTERM,
        "Sigterm variant must encode libc::SIGTERM"
    );
    assert_eq!(
        TestShutdownSignal::Sigint.as_libc_signal(),
        libc::SIGINT,
        "Sigint variant must encode libc::SIGINT"
    );
}

/// Delivers each variant with `kill_self` to a handler registered only for
/// its expected kind, so only a correctly mapped signal can wake it. Pins
/// the full chain variant → `as_libc_signal` → `libc::kill` → kernel →
/// handler; the existing shutdown cases use symmetric dual-branch selects
/// and cannot tell the two signals apart.
/// serial: signals are process-wide (STANDARDS §7); the 10s timeout is an
/// upper bound only — success path completes on delivery (STANDARDS §6 §9).
#[serial_test::serial]
#[tokio::test]
async fn test_kill_self_shutdown_signal_reaches_matching_handler() {
    // Independent oracle: which handler must observe each variant.
    let cases = [
        (TestShutdownSignal::Sigterm, SignalKind::terminate()),
        (TestShutdownSignal::Sigint, SignalKind::interrupt()),
    ];
    for (sig, kind) in cases {
        // Registration strictly precedes the send, so delivery is safe.
        let mut handler = signal(kind).expect("register signal handler");
        kill_self(sig);
        tokio::time::timeout(Duration::from_secs(10), handler.recv())
            .await
            .unwrap_or_else(|_| {
                panic!("kill_self({sig:?}) was not observed by its matching handler within 10s")
            })
            .expect("matching handler stream ended without delivering the signal");
    }
}
