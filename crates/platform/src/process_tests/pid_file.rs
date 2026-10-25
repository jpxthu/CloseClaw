use crate::process::{pid_file_path, pid_file_path_inner, read_pid_file, write_pid_file};
use tempfile::TempDir;

#[test]
fn test_write_and_read_pid_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");

    write_pid_file(&path, 12345).unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, Some(12345));
}

#[test]
fn test_read_pid_file_missing() {
    let path = std::path::Path::new("/nonexistent/daemon.pid");
    assert_eq!(read_pid_file(path), None);
}

#[test]
fn test_write_pid_file_overwrite() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");

    write_pid_file(&path, 111).unwrap();
    write_pid_file(&path, 222).unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, Some(222), "should read the latest written PID");
}

#[test]
fn test_write_pid_file_invalid_content() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("daemon.pid");

    // Manually write non-numeric content.
    std::fs::write(&path, "not_a_number").unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, None, "non-numeric PID file should return None");
}

#[test]
fn test_pid_file_path_inner() {
    let path = pid_file_path_inner("/home/user");
    assert_eq!(
        path,
        std::path::PathBuf::from("/home/user/.closeclaw/daemon.pid")
    );
}

#[test]
fn test_pid_file_path_inner_empty_home() {
    let path = pid_file_path_inner("");
    assert_eq!(path, std::path::PathBuf::from(".closeclaw/daemon.pid"));
}

// ── pid_file_path_inner pure computation tests ───────────────────────

/// pid_file_path_inner is pure path computation — no I/O side effects.
/// Running it twice with the same input must produce identical results,
/// and the path must NOT refer to a file that was created as a side effect.
#[test]
fn test_pid_file_path_inner_is_pure_no_io() {
    let path1 = pid_file_path_inner("/some/home");
    let path2 = pid_file_path_inner("/some/home");
    assert_eq!(path1, path2, "pure function must be idempotent");
    assert!(
        !path1.exists(),
        "pure function must not create files on disk"
    );
}

// ── pid_file_path() consistency tests ───────────────────────────────

/// pid_file_path() must return the same result as pid_file_path_inner(HOME)
/// — the public API delegates to the injectable inner, forming a single
/// source of truth for the fixed PID path.
#[test]
fn test_pid_file_path_matches_inner_with_home() {
    let public = pid_file_path().unwrap();
    let home = std::env::var("HOME").unwrap();
    let inner = pid_file_path_inner(&home);
    assert_eq!(
        public, inner,
        "pid_file_path() must equal pid_file_path_inner(HOME)"
    );
    assert!(public.is_absolute(), "fixed PID path must be absolute");
    assert!(public.to_string_lossy().ends_with("daemon.pid"));
}

/// pid_file_path() is a no-arg constant — multiple calls must always
/// return the same value, regardless of when or how often called.
#[test]
fn test_pid_file_path_is_stable_constant() {
    let first = pid_file_path().unwrap();
    let second = pid_file_path().unwrap();
    let third = pid_file_path().unwrap();
    assert_eq!(first, second);
    assert_eq!(second, third);
}
