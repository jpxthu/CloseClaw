//! Unit tests for the sidecar file lock behind `plan_file::acquire_plan_lock`.
//!
//! Verifies the advisory-lock semantics of the MSRV-compatible
//! `libc::flock` replacement for `std::fs::File::lock`: sidecar creation,
//! mutual exclusion across independent descriptors, release on drop,
//! blocking acquisition, and error propagation. All tests are serialized
//! (file-lock tests, docs/developer/STANDARDS.md §7).

use super::plan_archive;
use super::plan_file;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
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

// ── write_plan_file_locked ──────────────────────────────────────────────

/// Normal path: `write_plan_file_locked` to a fresh path creates the file
/// and its missing parent directories.
#[test]
#[serial_test::serial]
fn test_write_locked_creates_new_file_with_parent_dirs() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = dir.path().join("plans").join("nested").join("plan.md");

    plan_file::write_plan_file_locked(&plan, "# Title\n\n## Tasks\n")
        .expect("write to a fresh path must succeed");

    let content = std::fs::read_to_string(&plan).expect("new plan readable");
    assert_eq!(
        content, "# Title\n\n## Tasks\n",
        "new file must hold the exact written content"
    );
}

/// Normal path: `write_plan_file_locked` over an existing file replaces its
/// content entirely.
#[test]
#[serial_test::serial]
fn test_write_locked_overwrites_existing_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = write_plan(dir.path(), "plan.md");

    plan_file::write_plan_file_locked(&plan, "# Replaced\n").expect("overwrite must succeed");

    let content = std::fs::read_to_string(&plan).expect("plan readable");
    assert_eq!(
        content, "# Replaced\n",
        "existing file must be fully replaced by the new content"
    );
}

/// Mode preservation: a pre-existing file's permission mode survives the
/// atomic replace (chmod 0o600 before → still 0o600 after).
#[test]
#[serial_test::serial]
fn test_write_locked_preserves_existing_mode() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = write_plan(dir.path(), "plan.md");
    std::fs::set_permissions(&plan, std::fs::Permissions::from_mode(0o600))
        .expect("chmod 0o600 must succeed");

    plan_file::write_plan_file_locked(&plan, "# Secret\n").expect("write must succeed");

    let mode = std::fs::metadata(&plan)
        .expect("metadata readable")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "mode must stay 0o600 after the locked write"
    );
    assert_eq!(
        std::fs::read_to_string(&plan).expect("plan readable"),
        "# Secret\n",
        "content must be replaced while the mode is preserved"
    );
}

/// Error path: a parent path component occupied by a regular file makes the
/// write fail with `ENOTDIR` and no target is created. Pure path injection.
#[test]
#[serial_test::serial]
fn test_write_locked_errors_when_parent_is_a_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let occupied = dir.path().join("occupied");
    std::fs::write(&occupied, "just a file").expect("occupier written");
    let plan = occupied.join("sub").join("plan.md");

    let error = plan_file::write_plan_file_locked(&plan, "# Title\n")
        .expect_err("write must fail when a parent component is a file");

    assert_eq!(
        error.raw_os_error(),
        Some(libc::ENOTDIR),
        "file-occupied parent should surface as ENOTDIR, got {error:?}"
    );
    assert!(!plan.exists(), "failed write must not create the target");
}

/// Lock mutual exclusion: `write_plan_file_locked` blocks while an external
/// holder keeps the sidecar lock and completes once it is released — the
/// same blocking semantics as `acquire_plan_lock`.
#[test]
#[serial_test::serial]
fn test_write_locked_blocks_until_external_lock_released() {
    let dir = tempfile::TempDir::new().unwrap();
    let plan = write_plan(dir.path(), "plan.md");
    let held = plan_file::acquire_plan_lock(&plan).expect("holder acquires");

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (tx, rx) = mpsc::channel::<io::Result<()>>();
    let waiter_plan = plan.clone();
    let waiter = std::thread::spawn(move || {
        // Signal readiness *before* blocking on the lock so the main thread
        // only starts the negative assertion once this thread has actually
        // reached the locked write.
        let _ = ready_tx.send(());
        let result = plan_file::write_plan_file_locked(&waiter_plan, "# Blocked\n");
        let _ = tx.send(result);
    });

    ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("waiter thread must reach the locked write");

    // While the holder is alive the write must not complete. A timeout here
    // is the expected outcome; an early return means the lock is broken.
    match rx.recv_timeout(Duration::from_millis(250)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("waiter thread died before writing"),
        Ok(Ok(())) => panic!("locked write returned while the lock was still held"),
        Ok(Err(e)) => panic!("locked write failed while the lock was held: {e}"),
    }

    drop(held);
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("locked write failed after the holder released: {e}"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("locked write did not complete within 10s of the holder releasing")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("waiter thread panicked"),
    }
    waiter.join().expect("waiter thread must not panic");

    assert_eq!(
        std::fs::read_to_string(&plan).expect("plan readable"),
        "# Blocked\n",
        "the blocked write must land exactly once after the lock is released"
    );
}

/// Concurrency regression: interleaved `touch_access_timestamp` (session
/// read-modify-write under the lock) and `write_plan_file_locked` (full
/// overwrite under the same lock) keep the file intact across N rounds —
/// the access marker stays well-formed and no section tears.
#[test]
#[serial_test::serial]
fn test_touch_and_locked_write_interleave_without_tearing() {
    const ROUNDS: usize = 50;
    let full = "# Interleave\n\n<!-- accessed: 2026-01-01T00:00:00+00:00 -->\n\n## Context\n\nalpha\n\n## Tasks\n\n- [ ] a\n- [ ] b\n\n## Verification\n\ndone\n";

    let dir = tempfile::TempDir::new().unwrap();
    let plan = dir.path().join("plans").join("plan.md");
    plan_file::write_plan_file_locked(&plan, full).expect("seed write must succeed");

    let writer_plan = plan.clone();
    let writer = std::thread::spawn(move || {
        for _ in 0..ROUNDS {
            plan_file::write_plan_file_locked(&writer_plan, full)
                .expect("locked write must succeed in every round");
        }
    });
    let toucher_plan = plan.clone();
    let toucher = std::thread::spawn(move || {
        for _ in 0..ROUNDS {
            plan_file::touch_access_timestamp(&toucher_plan)
                .expect("touch must succeed in every round");
        }
    });
    writer.join().expect("writer thread must not panic");
    toucher.join().expect("toucher thread must not panic");

    let final_content = std::fs::read_to_string(&plan).expect("final plan readable");
    assert!(
        plan_file::parse_access_timestamp(&final_content).is_some(),
        "access marker must stay well-formed, got: {final_content}"
    );
    for fragment in [
        "## Context",
        "## Tasks",
        "## Verification",
        "- [ ] a",
        "- [ ] b",
    ] {
        assert!(
            final_content.contains(fragment),
            "fragment {fragment:?} must survive interleaved writes, got: {final_content}"
        );
    }

    let leftovers: Vec<String> = std::fs::read_dir(plan.parent().expect("plans dir"))
        .expect("plans dir readable")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".tmp."))
        .collect();
    assert!(
        leftovers.is_empty(),
        "atomic writes must not leave temp file leftovers, got: {leftovers:?}"
    );
}
