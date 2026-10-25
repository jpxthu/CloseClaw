use super::{is_signal_test_child, kill_self, TestSelfSignal};
use std::os::unix::process::ExitStatusExt;

// ── kill_self wrapper tests ────────────────────────────────────────

// The wrapper's normal path has two observable ends: the OS-level real
// delivery to the self process (verified here via the default disposition
// death signal in a subprocess) and the subscription-side observation
// (`test_subscribe_shutdown_signals_{sigterm,sigint}_child` assert the
// subscriber observes exactly the signal the wrapper sends). Together they
// pin 「signal sent by the wrapper == signal observed at the receiving end」.

/// Parent: spawns the child subprocess named `case` and asserts it died
/// from the real signal — `kill_self(sig)` must deliver the actual signal
/// to the process itself, not merely claim success.
fn assert_death_by_signal(sig: TestSelfSignal, case: &str) {
    let (want_signal, signal_name) = (sig.as_libc_signal(), sig.name());
    let test_binary = std::env::current_exe().expect("current exe");
    let output = std::process::Command::new(test_binary)
        .env("SIGNAL_TEST_CHILD", "1")
        .arg("--exact")
        .arg(format!("process_tests::kill_self::{case}"))
        .output()
        .expect("failed to run subprocess");
    assert_eq!(
        output.status.signal(),
        Some(want_signal),
        "kill_self({sig:?}) must terminate the un-subscribed process with \
         real {signal_name}, got: {:?}, child stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Subprocess child: delivers `sig` to itself through the wrapper with
/// no subscription installed — the default disposition must kill this
/// process, which the parent asserts via the exit status.
fn run_kill_self_child(sig: TestSelfSignal) {
    if !is_signal_test_child() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    kill_self(sig);
    // The signal's default disposition terminates the process before
    // this line; reaching it means the signal was not delivered.
    panic!("process survived kill_self({sig:?}) — signal not delivered");
}

/// Parent: spawns the child subprocess and asserts it died from SIGTERM.
#[test]
fn test_kill_self_sigterm_real_delivery() {
    assert_death_by_signal(
        TestSelfSignal::Sigterm,
        "test_kill_self_sigterm_real_delivery_child",
    );
}

/// Subprocess child: self-delivers SIGTERM through the wrapper.
#[test]
fn test_kill_self_sigterm_real_delivery_child() {
    run_kill_self_child(TestSelfSignal::Sigterm);
}

/// Parent: spawns the child subprocess and asserts it died from SIGINT.
#[test]
fn test_kill_self_sigint_real_delivery() {
    assert_death_by_signal(
        TestSelfSignal::Sigint,
        "test_kill_self_sigint_real_delivery_child",
    );
}

/// Subprocess child: self-delivers SIGINT through the wrapper.
#[test]
fn test_kill_self_sigint_real_delivery_child() {
    run_kill_self_child(TestSelfSignal::Sigint);
}
