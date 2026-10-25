use super::{in_signal_test_child, kill_self, open_child_subscription, TestSelfSignal};
use crate::process::ShutdownSignalSubscription;
use tokio::signal::unix::SignalKind;

// ── subscribe_shutdown_signals / next_signal tests ─────────────────

/// Parent: spawns the child subprocess named `case` under the
/// `--exact` filter (stdout discarded) and asserts it succeeded —
/// children run in subprocess isolation to avoid cross-test signal
/// handler contamination.
fn run_signal_child(case: &str) {
    let test_binary = std::env::current_exe().expect("current exe");
    let output = std::process::Command::new(test_binary)
        .env("SIGNAL_TEST_CHILD", "1")
        .arg("--exact")
        .arg(format!("process_tests::shutdown_signals::{case}"))
        .stdout(std::process::Stdio::null())
        .output()
        .expect("failed to run subprocess");
    assert!(
        output.status.success(),
        "{case} subprocess failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Arms a detached thread that self-delivers `sig` as soon as the
/// handshake token arrives on the returned sender. The caller releases
/// the sender only after `subscribe_shutdown_signals` returned — i.e.
/// after the OS handlers are installed and incoming signals are
/// buffered for the subscription — so delivery is deterministic and
/// cannot be lost, instead of relying on a sleep-timed race.
fn spawn_armed_kill(sig: TestSelfSignal) -> std::sync::mpsc::Sender<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        rx.recv().expect("subscribe handshake should arrive");
        kill_self(sig);
    });
    tx
}

/// One child round: release the armed sender for `sig`, await the
/// subscription's next signal, and assert the observed kind is `want`.
async fn deliver_and_expect(
    sub: &mut ShutdownSignalSubscription,
    sig: TestSelfSignal,
    want: SignalKind,
    context: &str,
) {
    let arm = spawn_armed_kill(sig);
    arm.send(()).expect("arming send should succeed");
    let kind = sub
        .next_signal()
        .await
        .expect("next_signal should return a signal");
    assert_eq!(kind, want, "{context}");
}

/// Child-side single-round body: skip unless spawned by a parent test,
/// then subscribe and deterministically deliver-and-verify one `sig`.
async fn subscribe_and_expect(sig: TestSelfSignal, want: SignalKind, context: &str) {
    if !in_signal_test_child() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    let mut sub = open_child_subscription().await;
    deliver_and_expect(&mut sub, sig, want, context).await;
}

/// Parent: runs the SIGTERM child and asserts the subprocess succeeds.
#[test]
fn test_subscribe_shutdown_signals_sigterm() {
    run_signal_child("test_subscribe_shutdown_signals_sigterm_child");
}

/// Subprocess child: sends SIGTERM and verifies next_signal returns it.
#[tokio::test]
async fn test_subscribe_shutdown_signals_sigterm_child() {
    subscribe_and_expect(
        TestSelfSignal::Sigterm,
        SignalKind::terminate(),
        "should return terminate",
    )
    .await;
}

/// Parent: runs the SIGINT child and asserts the subprocess succeeds.
#[test]
fn test_subscribe_shutdown_signals_sigint() {
    run_signal_child("test_subscribe_shutdown_signals_sigint_child");
}

/// Subprocess child: sends SIGINT and verifies next_signal returns it.
#[tokio::test]
async fn test_subscribe_shutdown_signals_sigint_child() {
    subscribe_and_expect(
        TestSelfSignal::Sigint,
        SignalKind::interrupt(),
        "should return interrupt",
    )
    .await;
}

/// Subscription can be called repeatedly: multiple next_signal calls
/// each return a subsequent signal in FIFO order.
/// Parent test spawns a subprocess with isolation to avoid cross-test
/// signal handler contamination.
#[test]
fn test_subscribe_shutdown_signals_multiple_next_signal() {
    run_signal_child("test_subscribe_shutdown_signals_multiple_next_signal_child");
}

/// Subprocess child: verifies repeated next_signal calls each capture
/// a subsequent signal — the subscription is reusable. Both rounds run
/// on the same subscription, each triggered deterministically via the
/// subscribe handshake.
#[tokio::test]
async fn test_subscribe_shutdown_signals_multiple_next_signal_child() {
    if !in_signal_test_child() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    let mut sub = open_child_subscription().await;

    // First signal: SIGTERM.
    deliver_and_expect(
        &mut sub,
        TestSelfSignal::Sigterm,
        SignalKind::terminate(),
        "first signal should be SIGTERM",
    )
    .await;

    // Second signal: SIGINT.
    deliver_and_expect(
        &mut sub,
        TestSelfSignal::Sigint,
        SignalKind::interrupt(),
        "second signal should be SIGINT",
    )
    .await;
}
