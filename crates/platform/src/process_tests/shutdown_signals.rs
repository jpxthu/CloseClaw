use super::{kill_self, TestSelfSignal};
use crate::process::subscribe_shutdown_signals;
use tokio::signal::unix::SignalKind;

// ── subscribe_shutdown_signals / next_signal tests ─────────────────

#[test]
fn test_subscribe_shutdown_signals_sigterm() {
    let test_binary = std::env::current_exe().expect("current exe");
    let output = std::process::Command::new(test_binary)
        .env("SIGNAL_TEST_CHILD", "1")
        .arg("--exact")
        .arg("process_tests::shutdown_signals::test_subscribe_shutdown_signals_sigterm_child")
        .output()
        .expect("failed to run subprocess");
    assert!(
        output.status.success(),
        "SIGTERM subprocess failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_subscribe_shutdown_signals_sigint() {
    let test_binary = std::env::current_exe().expect("current exe");
    let output = std::process::Command::new(test_binary)
        .env("SIGNAL_TEST_CHILD", "1")
        .arg("--exact")
        .arg("process_tests::shutdown_signals::test_subscribe_shutdown_signals_sigint_child")
        .output()
        .expect("failed to run subprocess");
    assert!(
        output.status.success(),
        "SIGINT subprocess failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Subprocess child: sends SIGTERM and verifies next_signal returns it.
#[tokio::test]
async fn test_subscribe_shutdown_signals_sigterm_child() {
    if std::env::var("SIGNAL_TEST_CHILD").is_err() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    let mut sub = subscribe_shutdown_signals()
        .await
        .expect("subscribe_shutdown_signals should succeed");
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        kill_self(TestSelfSignal::Sigterm);
    });
    let kind = sub
        .next_signal()
        .await
        .expect("next_signal should return a signal");
    assert_eq!(kind, SignalKind::terminate(), "should return terminate");
}

/// Subprocess child: sends SIGINT and verifies next_signal returns it.
#[tokio::test]
async fn test_subscribe_shutdown_signals_sigint_child() {
    if std::env::var("SIGNAL_TEST_CHILD").is_err() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    let mut sub = subscribe_shutdown_signals()
        .await
        .expect("subscribe_shutdown_signals should succeed");
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        kill_self(TestSelfSignal::Sigint);
    });
    let kind = sub
        .next_signal()
        .await
        .expect("next_signal should return a signal");
    assert_eq!(kind, SignalKind::interrupt(), "should return interrupt");
}

/// Subscription can be called repeatedly: multiple next_signal calls
/// each return a subsequent signal in FIFO order.
/// Parent test spawns a subprocess with isolation to avoid cross-test
/// signal handler contamination.
#[test]
fn test_subscribe_shutdown_signals_multiple_next_signal() {
    let test_binary = std::env::current_exe().expect("current exe");
    let output = std::process::Command::new(test_binary)
        .env("SIGNAL_TEST_CHILD", "1")
        .arg("--exact")
        .arg(
            "process_tests::shutdown_signals::\
             test_subscribe_shutdown_signals_multiple_next_signal_child",
        )
        .output()
        .expect("failed to run subprocess");
    assert!(
        output.status.success(),
        "multiple next_signal subprocess failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Subprocess child: verifies repeated next_signal calls each capture
/// a subsequent signal — the subscription is reusable.
#[tokio::test]
async fn test_subscribe_shutdown_signals_multiple_next_signal_child() {
    if std::env::var("SIGNAL_TEST_CHILD").is_err() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    let mut sub = subscribe_shutdown_signals()
        .await
        .expect("subscribe_shutdown_signals should succeed");

    // First signal: SIGTERM.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        kill_self(TestSelfSignal::Sigterm);
    });
    let first = sub
        .next_signal()
        .await
        .expect("first next_signal should return a signal");
    assert_eq!(
        first,
        SignalKind::terminate(),
        "first signal should be SIGTERM"
    );

    // Second signal: SIGINT.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        kill_self(TestSelfSignal::Sigint);
    });
    let second = sub
        .next_signal()
        .await
        .expect("second next_signal should return a signal");
    assert_eq!(
        second,
        SignalKind::interrupt(),
        "second signal should be SIGINT"
    );
}
