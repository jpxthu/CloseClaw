//! Re-exec helper scaffold for the `prepare_run` tests in `run_tests.rs`.
//!
//! Env mutation is prohibited by CONTRIBUTING.md §7, so env-dependent
//! behavior is tested by re-exec'ing this test binary as a gated helper
//! child with the target env injected (precedent:
//! crates/platform/src/process_tests.rs). The helper child wait is bounded
//! with a kill fallback (issue #3165). Split out of `run_tests.rs` to keep
//! that file under the 1000-line red line.

use super::run::prepare_run;
use closeclaw_platform::process::is_process_alive;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Env gate: set only on the re-exec'd child so normal test runs skip the
/// helper sub-test.
const HELPER_CHILD_GATE: &str = "PREPARE_RUN_HELPER_CHILD";

/// Raw `--config-dir` input injected by the parent test into the child.
const HELPER_CONFIG_DIR: &str = "PREPARE_RUN_HELPER_CONFIG_DIR";

/// Marker prefix of the resolved-path line printed by the helper child.
const HELPER_RESULT_MARKER: &str = "PREPARE_RUN_RESOLVED=";

/// Hard upper bound for one helper-child re-exec (issue #3165). A normal
/// re-exec completes well under 2s; 10s leaves headroom for slow starts
/// while staying under the 30s unit-test hard cap (STANDARDS.md §6).
const HELPER_CHILD_TIMEOUT: Duration = Duration::from_secs(10);

/// Wait bound injected by the hang-fault-injection test (issue #3165 Step
/// 1.3). Shorter than [`HELPER_CHILD_TIMEOUT`] so the case stays under the
/// 5s CI target (STANDARDS.md §6); the production-path bound is unchanged.
const TEST_HANG_INJECTION_TIMEOUT: Duration = Duration::from_secs(2);

/// Env gate for the hang fault injection: its value is the path of a pid
/// file (inside the parent's TempDir). When set alongside
/// [`HELPER_CHILD_GATE`], the helper child records its pid there and then
/// blocks forever, so the parent can exercise the timeout kill path.
const HELPER_CHILD_HANG: &str = "PREPARE_RUN_HELPER_HANG";

/// Poll interval of the helper-child watcher thread (bounded polling +
/// kill fallback precedent: crates/platform/src/process_tests.rs).
const HELPER_CHILD_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// `--exact` filter selecting only the helper sub-test inside the child.
///
/// Derived from `module_path!()` (issue #3161 Step 1.4 review) instead
/// of a hand-written string, so a module refactor cannot silently
/// desync the filter. libtest reports names *without* the leading crate
/// segment (`admin::run_helper_tests::…`) while `module_path!()` includes
/// it (`closeclaw_cli::admin::run_helper_tests::…`), so the crate-name
/// prefix is stripped here.
fn helper_child_test_name() -> String {
    let full = concat!(module_path!(), "::test_prepare_run_helper_child");
    full.strip_prefix(concat!(env!("CARGO_CRATE_NAME"), "::"))
        .unwrap_or(full)
        .to_string()
}

/// Re-exec the current test binary as the `prepare_run` helper child.
///
/// Raw entry keeping the explicit `remove_home` switch; call sites pick a
/// named wrapper ([`spawn_helper_child_without_home`] /
/// [`spawn_helper_child_with_home`]) so the intent is readable without the
/// signature (issue #3172-3).
///
/// `--exact` restricts the child to the helper sub-test; `envs` /
/// `remove_home` reproduce the isolation semantics of the old compiled
/// helper binary (subprocess-only env, no process-global mutation).
///
/// Unlike a bare `Command::output()`, the wait is bounded (issue #3165):
/// a watcher thread forwards the child's exit status over a channel and
/// the parent gives up after [`HELPER_CHILD_TIMEOUT`], killing and reaping
/// the child instead of hanging. `Err` carries the helper sub-test name
/// for diagnosis.
fn spawn_helper_child(
    config_dir: &str,
    envs: &[(&str, &str)],
    remove_home: bool,
) -> Result<std::process::Output, String> {
    spawn_helper_child_with_timeout(config_dir, envs, remove_home, HELPER_CHILD_TIMEOUT)
}

/// [`spawn_helper_child`] entry that removes `HOME` from the child env —
/// reproduces the home-unset failure path (e.g. `root_dir()` error) in a
/// subprocess instead of mutating the process-global env (CONTRIBUTING §7).
pub(super) fn spawn_helper_child_without_home(
    config_dir: &str,
    envs: &[(&str, &str)],
) -> Result<std::process::Output, String> {
    spawn_helper_child(config_dir, envs, true)
}

/// [`spawn_helper_child`] entry that keeps the parent's `HOME` in the child
/// env — the inverse of [`spawn_helper_child_without_home`] — so home
/// expansion cases resolve against the real parent home directory.
pub(super) fn spawn_helper_child_with_home(
    config_dir: &str,
    envs: &[(&str, &str)],
) -> Result<std::process::Output, String> {
    spawn_helper_child(config_dir, envs, false)
}

/// [`spawn_helper_child`] with an injectable wait bound — test-only hook so
/// the hang-fault-injection case can shrink the timeout without touching the
/// production-path bound. Call sites go through the named entry
/// [`spawn_helper_child_with_home_and_timeout`]; this raw form only plumbs
/// the `remove_home` switch between the wrappers.
fn spawn_helper_child_with_timeout(
    config_dir: &str,
    envs: &[(&str, &str)],
    remove_home: bool,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let sub_test = helper_child_test_name();
    let test_binary = std::env::current_exe().expect("failed to resolve current test binary");
    let mut cmd = std::process::Command::new(test_binary);
    cmd.env(HELPER_CHILD_GATE, "1")
        .env(HELPER_CONFIG_DIR, config_dir)
        .arg("--exact")
        .arg("--nocapture")
        .arg(&sub_test)
        // Same pipe semantics as `Command::output()`.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if remove_home {
        cmd.env_remove("HOME");
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn helper child `{sub_test}`: {e}"))?;

    // Drain the pipes on worker threads so the child can never deadlock on
    // a full pipe while the parent waits (what `output()` does internally).
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || drain_pipe(stdout_pipe));
    let stderr_reader = std::thread::spawn(move || drain_pipe(stderr_pipe));

    // Watcher thread: poll the child and forward its exit status so the
    // parent can wait with a timeout instead of blocking forever.
    let child = Arc::new(Mutex::new(child));
    let (tx, rx) = mpsc::channel();
    let watcher_child = Arc::clone(&child);
    std::thread::spawn(move || loop {
        let polled = watcher_child
            .lock()
            .expect("watcher: child lock")
            .try_wait();
        match polled {
            Ok(Some(status)) => {
                let _ = tx.send(Ok(status));
                return;
            }
            Ok(None) => std::thread::sleep(HELPER_CHILD_POLL_INTERVAL),
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        }
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(status)) => {
            let stdout = stdout_reader.join().unwrap_or_default();
            let stderr = stderr_reader.join().unwrap_or_default();
            Ok(std::process::Output {
                status,
                stdout,
                stderr,
            })
        }
        Ok(Err(e)) => Err(format!("helper child `{sub_test}` wait failed: {e}")),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Timed out: kill + reap so no child is left behind, then fail
            // with a diagnostic naming the helper sub-test (issue #3165).
            // wait() errors are ignored — the watcher may have reaped the
            // child first when it exits right at the timeout boundary.
            let mut child = child.lock().expect("timeout: child lock");
            let killed = child.kill().is_ok();
            let _ = child.wait();
            drop(child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            Err(format!(
                "helper child `{sub_test}` timed out after {}s and was killed \
                 (kill succeeded: {killed})",
                timeout.as_secs()
            ))
        }
        Err(e) => Err(format!("helper child `{sub_test}` watcher failed: {e}")),
    }
}

/// [`spawn_helper_child_with_home`] with an injectable wait bound — the
/// hang-fault-injection entry: home semantics are irrelevant to the timeout
/// path (the child keeps the parent's `HOME`), while the wait bound comes
/// from the caller so the case stays under the 5s CI target (STANDARDS.md
/// §6) without touching the production-path bound.
fn spawn_helper_child_with_home_and_timeout(
    config_dir: &str,
    envs: &[(&str, &str)],
    timeout: Duration,
) -> Result<std::process::Output, String> {
    spawn_helper_child_with_timeout(config_dir, envs, false, timeout)
}

/// Drain a child pipe to bytes on a worker thread; read errors yield
/// whatever was read so far (the consumers assert on content anyway).
fn drain_pipe<R: Read>(pipe: Option<R>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut buf);
    }
    buf
}

/// Helper sub-test body: resolves `prepare_run` under the parent-injected
/// env and prints the result with [`HELPER_RESULT_MARKER`]. Inert during a
/// normal run (gate unset); the parent tests drive it by re-exec'ing this
/// binary with [`HELPER_CHILD_GATE`] + `--exact` filtering.
#[test]
fn test_prepare_run_helper_child() {
    if std::env::var(HELPER_CHILD_GATE).is_err() {
        eprintln!("skipped: run via parent test subprocess");
        return;
    }
    // Hang fault injection (issue #3165 Step 1.3): record our pid for the
    // parent's residual-process check, then block forever. The wait is a
    // channel recv with a live but silent sender — no sleep polling
    // (STANDARDS.md §9); only the parent's kill can end it.
    if let Some(pid_file) = std::env::var(HELPER_CHILD_HANG).ok().map(PathBuf::from) {
        std::fs::write(&pid_file, std::process::id().to_string())
            .expect("failed to write hang-injection pid file");
        let (_tx, rx) = mpsc::channel::<()>();
        let _ = rx.recv();
        return;
    }
    let config_dir = std::env::var(HELPER_CONFIG_DIR).expect("missing config_dir input");
    let (resolved, _) = prepare_run(&config_dir, None).expect("prepare_run failed");
    println!("{HELPER_RESULT_MARKER}{}", resolved.display());
}

/// Run the helper child with optional env vars set and return the resolved
/// config_dir it printed (stripped from the [`HELPER_RESULT_MARKER`] line).
/// On failure, outputs both stdout and stderr for debugging.
pub(super) fn run_helper(config_dir: &str, envs: &[(&str, &str)]) -> String {
    let output = spawn_helper_child_with_home(config_dir, envs)
        .unwrap_or_else(|e| panic!("helper child did not complete: {e}"));
    assert!(
        output.status.success(),
        "helper child failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("helper output is not UTF-8");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(HELPER_RESULT_MARKER))
        .map(str::to_string)
        .unwrap_or_else(|| {
            panic!("helper child printed no {HELPER_RESULT_MARKER} line, stdout: {stdout}")
        })
}

/// Hang fault injection (issue #3165 Step 1.3): a helper child blocked
/// forever must surface as a timeout error within the wait bound — naming
/// the helper sub-test for diagnosis — with the child killed and reaped so
/// no process is left behind.
#[test]
fn test_spawn_helper_child_timeout_kills_hung_child() {
    let tmp = TempDir::new().expect("failed to create tempdir");
    let pid_file = tmp.path().join("helper-child.pid");

    let started = Instant::now();
    let result = spawn_helper_child_with_home_and_timeout(
        "",
        &[(
            HELPER_CHILD_HANG,
            pid_file.to_str().expect("utf-8 tempdir path"),
        )],
        TEST_HANG_INJECTION_TIMEOUT,
    );
    let elapsed = started.elapsed();

    // Bounded error path: the returned error is the timeout diagnostic
    // naming the helper sub-test, arriving near the injected bound (the
    // +2s margin absorbs kill + reap + pipe teardown under parallel load).
    let err = result.expect_err("hung helper child must surface a timeout error");
    assert!(
        err.contains("timed out"),
        "error should be the bounded-wait timeout, got: {err}"
    );
    assert!(
        err.contains(&helper_child_test_name()),
        "error should name the helper sub-test for diagnosis, got: {err}"
    );
    assert!(
        elapsed < TEST_HANG_INJECTION_TIMEOUT + Duration::from_secs(2),
        "wait must return near the timeout bound, took {elapsed:?}"
    );

    // No residual child: the timeout path killed and reaped it (pid file
    // proves the child reached the hang state before being killed).
    let child_pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("hang child should have written its pid file")
        .trim()
        .parse()
        .expect("pid file should contain a numeric pid");
    assert!(
        !is_process_alive(child_pid),
        "helper child {child_pid} should be reaped after the timeout kill"
    );
}
