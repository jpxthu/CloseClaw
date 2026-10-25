use super::{kill_self, TestSelfSignal};
use std::os::unix::process::ExitStatusExt;

// ── kill_self wrapper tests ────────────────────────────────────────

// The wrapper's normal path has two observable ends: the OS-level real
// delivery to the self process (verified here via the default disposition
// death signal in a subprocess) and the subscription-side observation
// (`test_subscribe_shutdown_signals_{sigterm,sigint}_child` assert the
// subscriber observes exactly the signal the wrapper sends). Together they
// pin 「signal sent by the wrapper == signal observed at the receiving end」.

/// Parent: spawns the child subprocess and asserts it died from SIGTERM —
/// `kill_self(TestSelfSignal::Sigterm)` must deliver the real SIGTERM to
/// the process itself, not merely claim success.
#[test]
fn test_kill_self_sigterm_real_delivery() {
    let test_binary = std::env::current_exe().expect("current exe");
    let output = std::process::Command::new(test_binary)
        .env("SIGNAL_TEST_CHILD", "1")
        .arg("--exact")
        .arg("process_tests::kill_self::test_kill_self_sigterm_real_delivery_child")
        .output()
        .expect("failed to run subprocess");
    assert_eq!(
        output.status.signal(),
        Some(libc::SIGTERM),
        "kill_self(Sigterm) must terminate the un-subscribed process with \
         real SIGTERM, got: {:?}, child stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Subprocess child: delivers SIGTERM to itself through the wrapper with
/// no subscription installed — the default disposition must kill this
/// process with SIGTERM, which the parent asserts via the exit status.
#[test]
fn test_kill_self_sigterm_real_delivery_child() {
    if std::env::var("SIGNAL_TEST_CHILD").is_err() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    kill_self(TestSelfSignal::Sigterm);
    // SIGTERM's default disposition terminates the process before this
    // line; reaching it means the signal was not delivered.
    panic!("process survived kill_self(Sigterm) — signal not delivered");
}

/// Parent: spawns the child subprocess and asserts it died from SIGINT —
/// `kill_self(TestSelfSignal::Sigint)` must deliver the real SIGINT to
/// the process itself, not merely claim success.
#[test]
fn test_kill_self_sigint_real_delivery() {
    let test_binary = std::env::current_exe().expect("current exe");
    let output = std::process::Command::new(test_binary)
        .env("SIGNAL_TEST_CHILD", "1")
        .arg("--exact")
        .arg("process_tests::kill_self::test_kill_self_sigint_real_delivery_child")
        .output()
        .expect("failed to run subprocess");
    assert_eq!(
        output.status.signal(),
        Some(libc::SIGINT),
        "kill_self(Sigint) must terminate the un-subscribed process with \
         real SIGINT, got: {:?}, child stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Subprocess child: delivers SIGINT to itself through the wrapper with
/// no subscription installed — the default disposition must kill this
/// process with SIGINT, which the parent asserts via the exit status.
#[test]
fn test_kill_self_sigint_real_delivery_child() {
    if std::env::var("SIGNAL_TEST_CHILD").is_err() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    kill_self(TestSelfSignal::Sigint);
    // SIGINT's default disposition terminates the process before this
    // line; reaching it means the signal was not delivered.
    panic!("process survived kill_self(Sigint) — signal not delivered");
}
