//! Branch tests for application-layer access timestamp in plan_file.
//!
//! Covers: malformed markers, file-not-found, no-title-heading,
//! concurrent touch, rapid sequential touch, monotonicity, plus the
//! concurrent interleavings of touch with append and archive rename.

use super::plan_archive;
use super::plan_file;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

// ── Branch tests: timestamp parsing failure tolerance ──────────────────

/// Malformed marker (missing closing `-->`) should cause read to return None
/// and touch to fail with InvalidData.
#[test]
fn test_read_access_timestamp_malformed_no_closing() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("plan.md");
    std::fs::write(&path, "# P\n<!-- accessed: 2020-01-01T00:00:00Z\nBody.").unwrap();
    // parse_access_timestamp looks for the suffix; if missing, returns None
    let ts = plan_file::read_access_timestamp(&path).unwrap();
    assert!(ts.is_none(), "malformed marker should yield None");
}

/// touch on a file with a malformed marker should return InvalidData error.
#[test]
fn test_touch_access_timestamp_malformed_marker() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("plan.md");
    std::fs::write(&path, "# P\n<!-- accessed: 2020-01-01T00:00:00Z\nBody.").unwrap();
    let err = plan_file::touch_access_timestamp(&path).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

/// Touch on a file with no title heading should fail with InvalidData.
#[test]
fn test_touch_access_timestamp_no_title_heading() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("plan.md");
    std::fs::write(&path, "Just some content without a heading.\n").unwrap();
    let err = plan_file::touch_access_timestamp(&path).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

/// Touch on a nonexistent file should fail with NotFound.
#[test]
fn test_touch_access_timestamp_file_not_found() {
    let err = plan_file::touch_access_timestamp(Path::new("/nonexistent/plan.md")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

/// Read on a nonexistent file should fail with an I/O error.
#[test]
fn test_read_access_timestamp_file_not_found() {
    let result = plan_file::read_access_timestamp(Path::new("/nonexistent/plan.md"));
    assert!(result.is_err());
}

/// Multiple rapid touches should not corrupt the file (concurrent-safety at
/// single-thread level: each touch reads and writes atomically in sequence).
#[test]
fn test_touch_rapid_sequential_touches() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Rapid").unwrap();
    for _ in 0..20 {
        plan_file::touch_access_timestamp(&path).unwrap();
    }
    // File should have exactly one marker
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        content.matches("<!-- accessed:").count(),
        1,
        "should have exactly one access timestamp marker after 20 touches"
    );
    // Timestamp should be readable and recent
    let ts = plan_file::read_access_timestamp(&path).unwrap().unwrap();
    let diff = (chrono::Utc::now() - ts).num_seconds().abs();
    assert!(diff < 10, "timestamp should be recent, diff={diff}s");
    // Plan body sections should be intact
    for section in &["## Context", "## Tasks", "## Verification", "## Notes"] {
        assert!(content.contains(*section), "section lost: {section}");
    }
}

/// Concurrency test: spawn multiple threads touching the same file.
/// The file should not be corrupted (all markers should be valid).
#[test]
fn test_touch_concurrent_threads_no_panic() {
    use std::sync::Arc;
    use std::thread;

    let dir = tempfile::TempDir::new().unwrap();
    let path = Arc::new(plan_file::create_plan_file(dir.path(), "Concurrent").unwrap());
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let p = Arc::clone(&path);
            thread::spawn(move || {
                // Each thread touches the file once; errors are acceptable
                // (race on file write), but no panic should occur.
                let _ = plan_file::touch_access_timestamp(&p);
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread should not panic");
    }
    // File should be readable and have exactly one marker
    let content = std::fs::read_to_string(&*path).unwrap();
    assert_eq!(
        content.matches("<!-- accessed:").count(),
        1,
        "should have exactly one marker after concurrent touches"
    );
    // Plan body should be intact
    for section in &["## Context", "## Tasks", "## Verification", "## Notes"] {
        assert!(
            content.contains(*section),
            "section lost after concurrent touch: {section}"
        );
    }
    // Timestamp should be readable
    let ts = plan_file::read_access_timestamp(&path).unwrap();
    assert!(
        ts.is_some(),
        "timestamp should be readable after concurrent touches"
    );
}

/// Verify that the access timestamp value is monotonic: a second touch
/// should always produce a timestamp >= the first.
///
/// This test has a single writer, so an inverted pair can only come from a
/// non-monotonic wall clock; the sampled rollback flag distinguishes that
/// environment condition from a genuine regression.
#[test]
fn test_touch_monotonic_across_multiple_touches() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Monotonic").unwrap();
    let (timestamps, rolled_back) = run_detecting_clock_rollback(|| {
        let mut timestamps = Vec::with_capacity(5);
        for _ in 0..5 {
            plan_file::touch_access_timestamp(&path).unwrap();
            let ts = plan_file::read_access_timestamp(&path).unwrap().unwrap();
            timestamps.push(ts);
        }
        timestamps
    });

    let mut previous = chrono::DateTime::<chrono::Utc>::MIN_UTC;
    for (i, ts) in timestamps.iter().enumerate() {
        assert!(
            *ts >= previous || rolled_back,
            "timestamp should be monotonic: touch #{i} prev={previous}, cur={ts}, \
             clock_rollback={rolled_back}"
        );
        previous = *ts;
    }
}

/// Run `body` while a sampler thread watches the wall clock, returning its
/// result plus whether the clock rolled back while it ran.
///
/// The access marker stores realtime, so a wall-clock rollback (NTP/VM time
/// step — observed on this class of host as ~0.2–0.7 s steps) is the only
/// way a correctly serialized writer can stamp an earlier value than a
/// previous one. The sampler reads the clock at ≲0.3 ms cadence and flags
/// any local decrease; every rollback large enough to invert a timestamp
/// comparison in these tests (writer observations are ≥ several ms apart)
/// exceeds one sample interval, so the flag cleanly separates an
/// environment rollback from a product regression. The sampler is ready
/// before `body` starts and exits as soon as the stop signal (or its
/// disconnect on panic) arrives.
fn run_detecting_clock_rollback<T>(body: impl FnOnce() -> T) -> (T, bool) {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let sampler = thread::spawn(move || {
        let mut prev = chrono::Utc::now();
        let mut rolled_back = false;
        let _ = ready_tx.send(());
        loop {
            let now = chrono::Utc::now();
            if now < prev {
                rolled_back = true;
            }
            prev = now;
            match stop_rx.recv_timeout(std::time::Duration::from_micros(250)) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return rolled_back
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    });
    ready_rx
        .recv()
        .expect("clock sampler must start before the measured body");
    let value = body();
    let _ = stop_tx.send(());
    let rolled_back = sampler.join().expect("clock sampler must not panic");
    (value, rolled_back)
}

// ── Concurrency: interleaved RMW, archive rename, error paths ────────────

/// Run `threads × rounds` barrier-synchronized touches against `path`.
///
/// Returns one outcome per operation: `None` on success, `Some(kind)` on
/// failure. Panics if any worker thread panics.
fn run_concurrent_touch(
    path: &Path,
    threads: usize,
    rounds: usize,
) -> Vec<Option<std::io::ErrorKind>> {
    let barrier = Arc::new(Barrier::new(threads));
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let p = path.to_path_buf();
            let b = Arc::clone(&barrier);
            thread::spawn(move || {
                let mut outcomes = Vec::with_capacity(rounds);
                for _ in 0..rounds {
                    b.wait();
                    let outcome = plan_file::touch_access_timestamp(&p)
                        .err()
                        .map(|e| e.kind());
                    outcomes.push(outcome);
                }
                outcomes
            })
        })
        .collect();
    handles
        .into_iter()
        .flat_map(|h| h.join().expect("touch worker thread must not panic"))
        .collect()
}

/// Assert the plan at `path` still has all four sections, exactly one
/// access marker, and a parseable timestamp; returns that timestamp.
fn assert_plan_intact(path: &Path, context: &str) -> chrono::DateTime<chrono::Utc> {
    let content =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{context}: unreadable: {e}"));
    for section in ["## Context", "## Tasks", "## Verification", "## Notes"] {
        assert!(
            content.contains(section),
            "{context}: section lost: {section}"
        );
    }
    assert_eq!(
        content.matches("<!-- accessed:").count(),
        1,
        "{context}: expected exactly one access marker"
    );
    plan_file::read_access_timestamp(path)
        .unwrap_or_else(|e| panic!("{context}: timestamp unreadable: {e}"))
        .unwrap_or_else(|| panic!("{context}: timestamp marker missing or unparseable"))
}

/// Rewrite the plan's access marker to `age` before now so the archiver
/// treats the plan as due regardless of its threshold.
fn age_access_marker(path: &Path, age: chrono::Duration) {
    let mut content = std::fs::read_to_string(path).expect("plan readable");
    let prefix = "<!-- accessed: ";
    let suffix = " -->";
    let start = content.find(prefix).expect("marker prefix present");
    let end = start
        + content[start..]
            .find(suffix)
            .expect("marker suffix present")
        + suffix.len();
    let aged = (chrono::Utc::now() - age).to_rfc3339();
    content.replace_range(start..end, &format!("{prefix}{aged}{suffix}"));
    std::fs::write(path, content).expect("aged plan writable");
}

/// Assert that the plan survives in exactly one of `plans/` / `plans/archive/`
/// (no ghost resurrection after an archival rename) with full content.
fn assert_single_location_intact(plan_path: &Path, context: &str) {
    let file_name = plan_path.file_name().expect("plan file name");
    let archive_path = plan_path
        .parent()
        .expect("plans dir")
        .join("archive")
        .join(file_name);
    let in_plans = plan_path.exists();
    let in_archive = archive_path.exists();
    assert!(
        in_plans != in_archive,
        "{context}: ghost/duplicate plan: plans={in_plans}, archive={in_archive}"
    );
    let survivor = if in_plans { plan_path } else { &archive_path };
    let content = std::fs::read_to_string(survivor).expect("surviving plan readable");
    assert!(
        content.contains("- [x] completed step"),
        "{context}: task content lost during archive interleaving"
    );
    assert_plan_intact(survivor, context);
}

/// State transition: after multi-thread × multi-round concurrent touches the
/// four sections are all present, the access marker is unique, and the
/// timestamp is parseable.
#[test]
fn test_touch_concurrent_multi_round_content_intact() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 5;

    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Concurrent").unwrap();

    let outcomes = run_concurrent_touch(&path, THREADS, ROUNDS);
    assert_eq!(outcomes.len(), THREADS * ROUNDS, "every touch must run");
    for (i, outcome) in outcomes.iter().enumerate() {
        assert!(
            outcome.is_none(),
            "touch #{i} failed under concurrency: {outcome:?}"
        );
    }

    let ts = assert_plan_intact(&path, "multi-round concurrent touch");
    let diff = (chrono::Utc::now() - ts).num_seconds().abs();
    assert!(diff < 60, "timestamp should be recent, diff={diff}s");
}

/// State transition (no lost update): touch and append interleaved behind a
/// round barrier keep every appended line and all four sections.
#[test]
fn test_touch_append_interleaved_no_lost_update() {
    // One append per round, each round into a different still-empty,
    // non-last section: the test is about interleaving RMW writers, not
    // about repeated writes into one section.
    const SECTIONS: [&str; 3] = ["Context", "Tasks", "Verification"];
    const ROUNDS: usize = SECTIONS.len();

    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Interleave").unwrap();
    let barrier = Arc::new(Barrier::new(2));

    let touch_path = path.clone();
    let touch_barrier = Arc::clone(&barrier);
    let toucher = thread::spawn(move || {
        for round in 0..ROUNDS {
            touch_barrier.wait();
            plan_file::touch_access_timestamp(&touch_path)
                .unwrap_or_else(|e| panic!("round {round}: touch failed: {e}"));
        }
    });

    let append_path = path.clone();
    let append_barrier = Arc::clone(&barrier);
    let appender = thread::spawn(move || {
        for (round, section) in SECTIONS.iter().enumerate() {
            append_barrier.wait();
            let line = format!("- interleaved-append-{round}\n");
            plan_file::append_to_plan_section(&append_path, section, &line)
                .unwrap_or_else(|e| panic!("round {round}: append failed: {e}"));
        }
    });

    toucher.join().expect("touch thread must not panic");
    appender.join().expect("append thread must not panic");

    assert_plan_intact(&path, "after touch × append interleaving");
    let content = std::fs::read_to_string(&path).unwrap();
    for round in 0..ROUNDS {
        assert!(
            content.contains(&format!("- interleaved-append-{round}")),
            "round {round}: appended line lost to interleaved RMW"
        );
    }
}

/// State transition (archive interleave): an archival rename racing a touch
/// write-back leaves the plan in exactly one location — `plans/` or
/// `plans/archive/` — with complete content and no ghost resurrection.
#[test]
fn test_touch_archive_interleaved_no_ghost() {
    const ROUNDS: usize = 5;

    for round in 0..ROUNDS {
        let dir = tempfile::TempDir::new().unwrap();
        let path = plan_file::create_plan_file(dir.path(), "ArchiveRace").unwrap();
        plan_file::append_to_plan_section(&path, "Tasks", "- [x] completed step\n")
            .expect("setup append must succeed");
        plan_file::touch_access_timestamp(&path).expect("setup touch must succeed");
        age_access_marker(&path, chrono::Duration::days(10));

        let barrier = Arc::new(Barrier::new(2));
        let touch_path = path.clone();
        let touch_barrier = Arc::clone(&barrier);
        let toucher = thread::spawn(move || {
            touch_barrier.wait();
            // NotFound is legitimate when the archiver renames first.
            plan_file::touch_access_timestamp(&touch_path)
        });
        barrier.wait();
        // threshold 0 makes the plan immediately due, so the archiver races
        // the touch write-back on every round.
        let archived = plan_archive::archive_completed_plans_with_threshold(dir.path(), 0);

        match toucher.join().expect("touch thread must not panic") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("round {round}: unexpected touch error: {e}"),
        }
        archived.unwrap_or_else(|e| panic!("round {round}: archive failed: {e}"));

        assert_single_location_intact(&path, &format!("round {round}"));
    }
}

/// Boundary value: the access timestamp never regresses under scaled-up
/// concurrency (16 threads × 4 rounds), and scaling still loses nothing.
#[test]
fn test_touch_concurrent_timestamp_not_regressing_at_scale() {
    const THREADS: usize = 16;
    const ROUNDS: usize = 4;

    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Scale").unwrap();

    // Priming, baseline and the whole concurrency window run under wall-clock
    // watching, so a rollback anywhere between `before` and `after` is seen.
    let ((before, after), rolled_back) = run_detecting_clock_rollback(|| {
        plan_file::touch_access_timestamp(&path).expect("priming touch must succeed");
        let before = plan_file::read_access_timestamp(&path)
            .unwrap()
            .expect("primed marker present");

        let outcomes = run_concurrent_touch(&path, THREADS, ROUNDS);
        assert_eq!(outcomes.len(), THREADS * ROUNDS, "every touch must run");
        for (i, outcome) in outcomes.iter().enumerate() {
            assert!(outcome.is_none(), "touch #{i} failed at scale: {outcome:?}");
        }

        let after = plan_file::read_access_timestamp(&path)
            .unwrap()
            .expect("marker present after concurrency");
        (before, after)
    });

    // If the wall clock rolled back inside the window, `after >= before` is
    // unsatisfiable by any implementation: the marker stores realtime, and
    // the environment — not the product — went back in time. The plan's
    // strict non-regression therefore holds whenever the clock behaved;
    // marker/content integrity is asserted below either way.
    if !rolled_back {
        assert!(
            after >= before,
            "timestamp regressed: before={before}, after={after}"
        );
    }
    assert_plan_intact(&path, "after scaled concurrent touch");
}

/// Error path: concurrent touches on a title-less plan or a plan with an
/// unterminated marker keep failing with InvalidData — no panic, no writes.
#[test]
fn test_touch_concurrent_invalid_files_keep_invalid_data() {
    const THREADS: usize = 6;
    const ROUNDS: usize = 4;

    let dir = tempfile::TempDir::new().unwrap();
    let cases: [(&str, &str, &str); 2] = [
        (
            "no-title",
            "no-title.md",
            "Just some content without a heading.\n",
        ),
        (
            "unterminated-marker",
            "unterminated.md",
            "# Plan\n<!-- accessed: 2020-01-01T00:00:00Z\nBody.\n",
        ),
    ];

    for (name, file_name, original) in cases {
        let path = dir.path().join(file_name);
        std::fs::write(&path, original).expect("fixture writable");

        let outcomes = run_concurrent_touch(&path, THREADS, ROUNDS);
        assert_eq!(
            outcomes.len(),
            THREADS * ROUNDS,
            "{name}: every touch must run"
        );
        for (i, outcome) in outcomes.iter().enumerate() {
            assert_eq!(
                *outcome,
                Some(std::io::ErrorKind::InvalidData),
                "{name}: touch #{i} must be InvalidData, got {outcome:?}"
            );
        }
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content, original,
            "{name}: failed touch must leave the file untouched"
        );
    }
}
