use crate::platform::process::{pid_file_path, read_pid_file, write_pid_file};
use tempfile::TempDir;

#[test]
fn test_write_and_read_pid_file() {
    let tmp = TempDir::new().unwrap();
    let path = pid_file_path(tmp.path());

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
    let path = pid_file_path(tmp.path());

    write_pid_file(&path, 111).unwrap();
    write_pid_file(&path, 222).unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, Some(222), "should read the latest written PID");
}

#[test]
fn test_write_pid_file_invalid_content() {
    let tmp = TempDir::new().unwrap();
    let path = pid_file_path(tmp.path());

    // Manually write non-numeric content.
    std::fs::write(&path, "not_a_number").unwrap();
    let pid = read_pid_file(&path);
    assert_eq!(pid, None, "non-numeric PID file should return None");
}

#[test]
fn test_pid_file_path_format() {
    let dir = std::path::Path::new("/tmp/test");
    let path = pid_file_path(dir);
    assert_eq!(path, std::path::PathBuf::from("/tmp/test/daemon.pid"));
}
