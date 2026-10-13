//! Unit tests for the sidecar file lock behind `plan_file::acquire_plan_lock`.
//!
//! Verifies the advisory-lock semantics of the MSRV-1.82-compatible
//! `libc::flock` replacement for `std::fs::File::lock`: sidecar creation,
//! mutual exclusion across independent descriptors, release on drop,
//! blocking acquisition, and error propagation. All tests are serialized
//! (file-lock tests, docs/developer/STANDARDS.md §7).

use super::plan_archive;
use super::plan_file;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

// ── Helpers ─────────────────────────────────────────────────────────────

/// Build the sidecar lock path for `plan` the way `acquire_plan_lock` does.
fn sidecar_path(plan: &Path) -> PathBuf {
    let mut name = plan.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

/// Minimal plan file fixture at `{dir}/{name}`.
fn write_plan(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "# Test Plan\n\n## Tasks\n\n- [ ] todo\n")
        .expect("plan fixture writable");
    path
}

/// Non-blocking exclusive `flock` probe on a fresh, independent descriptor.
///
/// `Ok(())` means no other descriptor holds an exclusive lock on the
/// sidecar; `EWOULDBLOCK` means some other descriptor does.
fn probe_exclusive(sidecar: &Path) -> io::Result<()> {
    let file = File::open(sidecar).expect("sidecar lock file readable");
    // SAFETY: `file` is a valid open descriptor that outlives the call;
    // `flock` only updates kernel lock state for that descriptor and
    // dereferences no user memory.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Assert the probe is refused with `EWOULDBLOCK` (lock held elsewhere).
fn assert_probe_blocked(sidecar: &Path, context: &str) {
    let error = match probe_exclusive(sidecar) {
        Ok(()) => panic!("{context}: probe should be blocked while the lock is held"),
        Err(e) => e,
    };
    assert_eq!(
        error.raw_os_error(),
        Some(libc::EWOULDBLOCK),
        "{context}: expected EWOULDBLOCK from probe, got {error:?}"
    );
}

/// Rewrite the plan's access marker to `days` before now so the archiver
/// treats the plan as due regardless of its threshold.
fn age_access_marker(path: &Path, days: i64) {
    let mut content = std::fs::read_to_string(path).expect("plan readable");
    let prefix = plan_file::ACCESS_TIMESTAMP_MARKER_PREFIX;
    let suffix = plan_file::ACCESS_TIMESTAMP_MARKER_SUFFIX;
    let start = content.find(prefix).expect("marker prefix present");
    let end = start
        + content[start..]
            .find(suffix)
            .expect("marker suffix present")
        + suffix.len();
    let aged = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    content.replace_range(start..end, &format!("{prefix}{aged}{suffix}"));
    std::fs::write(path, content).expect("aged plan writable");
}

// ── Normal path ─────────────────────────────────────────────────────────

/// Normal path: `acquire_plan_lock` creates the `{plan}.lock` sidecar and
/// returns the locked `File`.
#[test]
#[serial_test::serial]
fn test_acquire_creates_sidecar_and_returns_lock() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = write_plan(dir.path(), "plan.md");

    let lock = plan_file::acquire_plan_lock(&plan).expect("free lock must be acquirable");

    let sidecar = sidecar_path(&plan);
    assert!(
        sidecar.exists(),
        "sidecar lock file should be created at {}",
        sidecar.display()
    );
    drop(lock);
}

/// Regression: the read-modify-write (`mutate_plan_file`) and archive code
/// paths keep working under the flock-based lock.
#[test]
#[serial_test::serial]
fn test_mutate_and_archive_work_under_lock() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Locked").expect("plan created");

    plan_file::update_plan_timestamp(path.to_str().expect("utf-8 plan path"))
        .expect("update_plan_timestamp must succeed under the new lock");
    plan_file::append_to_plan_section(&path, "Tasks", "- [x] done\n")
        .expect("append_to_plan_section must succeed under the new lock");
    let content = std::fs::read_to_string(&path).expect("plan readable");
    assert!(
        content.contains("- [x] done"),
        "appended task line should be present in the plan"
    );

    plan_file::touch_access_timestamp(&path).expect("touch must succeed under the new lock");
    age_access_marker(&path, 10);
    let archived =
        plan_archive::archive_completed_plans_with_threshold(dir.path(), 7).expect("archive path");
    assert_eq!(
        archived, 1,
        "the completed, aged plan should be archived through the same lock domain"
    );
    assert!(!path.exists(), "archived plan should leave plans/");
}

// ── Exclusivity and release ─────────────────────────────────────────────

/// Exclusivity: while the returned `File` is alive, an independent
/// descriptor's non-blocking exclusive probe gets `EWOULDBLOCK`; once the
/// holder drops, the same probe succeeds.
#[test]
#[serial_test::serial]
fn test_exclusive_lock_blocks_independent_probe_while_held() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = write_plan(dir.path(), "plan.md");
    let sidecar = sidecar_path(&plan);

    let lock = plan_file::acquire_plan_lock(&plan).expect("lock acquired");
    assert_probe_blocked(&sidecar, "holder alive");
    drop(lock);
    probe_exclusive(&sidecar).expect("probe must succeed after the holder released the lock");
}

/// Release: dropping the `File` returned by `acquire_plan_lock` releases the
/// lock, so a fresh probe and a fresh acquisition both succeed.
#[test]
#[serial_test::serial]
fn test_lock_released_when_returned_file_dropped() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = write_plan(dir.path(), "plan.md");
    let sidecar = sidecar_path(&plan);

    let lock = plan_file::acquire_plan_lock(&plan).expect("first acquisition");
    assert_probe_blocked(&sidecar, "first holder alive");
    drop(lock);

    probe_exclusive(&sidecar).expect("probe must succeed after drop");
    let relock = plan_file::acquire_plan_lock(&plan).expect("re-acquisition after release");
    drop(relock);
}

/// Blocking semantics: a second `acquire_plan_lock` on the same sidecar does
/// not return while the lock is held, and completes once the holder drops.
#[test]
#[serial_test::serial]
fn test_blocking_acquire_waits_for_holder_release() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = write_plan(dir.path(), "plan.md");
    let held = plan_file::acquire_plan_lock(&plan).expect("holder acquires");

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (tx, rx) = mpsc::channel::<io::Result<File>>();
    let waiter_plan = plan.clone();
    let waiter = std::thread::spawn(move || {
        // Signal readiness *before* blocking on the lock so the main thread
        // only starts the negative assertion once this thread has actually
        // reached `acquire_plan_lock` (distinguishes "not scheduled yet"
        // from "scheduled and blocked on the lock").
        let _ = ready_tx.send(());
        let result = plan_file::acquire_plan_lock(&waiter_plan);
        let _ = tx.send(result);
    });

    ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("waiter thread must reach acquire_plan_lock");

    // While the holder is alive the waiter must not return. A timeout here
    // is the expected outcome; an early return means the lock is broken.
    match rx.recv_timeout(Duration::from_millis(250)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("waiter thread died before acquiring"),
        Ok(Ok(_)) => panic!("blocking acquire returned while the lock was still held"),
        Ok(Err(e)) => panic!("waiter acquire failed while the lock was held: {e}"),
    }

    drop(held);
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(_waiter_lock)) => {}
        Ok(Err(e)) => panic!("waiter acquire failed after the holder released: {e}"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("waiter did not acquire within 10s of the holder releasing")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("waiter thread panicked"),
    }
    waiter.join().expect("waiter thread must not panic");
}

// ── Error path ──────────────────────────────────────────────────────────

/// Error path: a `plan_path` whose parent directory does not exist makes
/// sidecar creation fail, and the `io::Error` propagates as `NotFound`
/// out of `acquire_plan_lock`. Pure path injection — no permission games.
#[test]
#[serial_test::serial]
fn test_acquire_errors_when_sidecar_parent_is_missing() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = dir.path().join("no-such-dir").join("plan.md");

    let error =
        plan_file::acquire_plan_lock(&plan).expect_err("acquire must fail for a missing parent");

    assert_eq!(
        error.kind(),
        io::ErrorKind::NotFound,
        "uncreatable sidecar should surface as NotFound, got {error:?}"
    );
}

/// Error path: when `{plan}.lock` is already a directory, sidecar creation
/// fails with `EISDIR` and the raw OS error propagates. Pure path injection
/// — no `chmod`, no `set_readonly`, no lint allowances.
#[test]
#[serial_test::serial]
fn test_acquire_errors_when_sidecar_slot_is_a_directory() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = write_plan(dir.path(), "plan.md");
    let sidecar = sidecar_path(&plan);
    std::fs::create_dir(&sidecar).expect("sidecar slot occupied by a directory");

    let error =
        plan_file::acquire_plan_lock(&plan).expect_err("acquire must fail for a directory slot");

    assert_eq!(
        error.raw_os_error(),
        Some(libc::EISDIR),
        "occupied sidecar slot should surface as EISDIR, got {error:?}"
    );
}
