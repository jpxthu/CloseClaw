//! Branch tests for application-layer access timestamp in plan_file.
//!
//! Covers: malformed markers, file-not-found, no-title-heading,
//! concurrent touch, rapid sequential touch, monotonicity.

use super::plan_file;
use std::path::Path;

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
    let ts = plan_file::read_access_timestamp(&*path).unwrap();
    assert!(
        ts.is_some(),
        "timestamp should be readable after concurrent touches"
    );
}

/// Verify that the access timestamp value is monotonic: a second touch
/// should always produce a timestamp >= the first.
#[test]
fn test_touch_monotonic_across_multiple_touches() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = plan_file::create_plan_file(dir.path(), "Monotonic").unwrap();
    let mut previous = chrono::DateTime::<chrono::Utc>::MIN_UTC;
    for _ in 0..5 {
        plan_file::touch_access_timestamp(&path).unwrap();
        let ts = plan_file::read_access_timestamp(&path).unwrap().unwrap();
        assert!(
            ts >= previous,
            "timestamp should be monotonic: prev={previous}, cur={ts}"
        );
        previous = ts;
    }
}
