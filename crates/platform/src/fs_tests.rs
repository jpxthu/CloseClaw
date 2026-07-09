use crate::fs::{check_executable, check_readable, check_writable, expand_home, normalize_path};
use std::path::{Path, PathBuf};

#[test]
fn test_normalize_path_unix() {
    let path = Path::new("/usr/local/bin");
    let normalized = normalize_path(path);
    assert_eq!(normalized, PathBuf::from("/usr/local/bin"));
}

#[test]
fn test_normalize_path_backslashes() {
    let path = Path::new(r"C:\Users\test\file.txt");
    let normalized = normalize_path(path);
    assert_eq!(normalized, PathBuf::from("C:/Users/test/file.txt"));
}

#[test]
fn test_normalize_path_mixed_separators() {
    let path = Path::new(r"C:\Users/test\another/file");
    let normalized = normalize_path(path);
    assert_eq!(normalized, PathBuf::from("C:/Users/test/another/file"));
}

#[test]
fn test_normalize_path_already_normalized() {
    let path = Path::new("/a/b/c");
    let normalized = normalize_path(path);
    assert_eq!(normalized, PathBuf::from("/a/b/c"));
}

#[test]
fn test_normalize_path_empty() {
    let path = Path::new("");
    let normalized = normalize_path(path);
    assert_eq!(normalized, PathBuf::from(""));
}

#[test]
fn test_normalize_path_trailing_separator() {
    let path = Path::new(r"C:\Users\test\");
    let normalized = normalize_path(path);
    assert_eq!(normalized, PathBuf::from("C:/Users/test/"));
}

#[test]
fn test_expand_home_tilde() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/test".to_string());
    let result = expand_home(Path::new("~/foo"));
    assert_eq!(result, PathBuf::from(home).join("foo"));
}

#[test]
fn test_expand_home_tilde_no_slash() {
    let result = expand_home(Path::new("~"));
    assert_eq!(result, PathBuf::from("~"));
}

#[test]
fn test_expand_home_unknown_var() {
    let result = expand_home(Path::new("%UNKNOWN_VAR%/foo"));
    assert_eq!(result, PathBuf::from("%UNKNOWN_VAR%/foo"));
}

#[test]
fn test_expand_home_empty_var_name() {
    let result = expand_home(Path::new("%%/foo"));
    assert_eq!(result, PathBuf::from("%%/foo"));
}

#[test]
fn test_expand_home_appdata() {
    // If APPDATA is set (Windows), verify expansion works
    // On Linux it won't be set, so just verify fallback
    if let Ok(appdata) = std::env::var("APPDATA") {
        let result = expand_home(Path::new("%APPDATA%/foo"));
        assert_eq!(result, PathBuf::from(appdata).join("foo"));
    } else {
        let result = expand_home(Path::new("%APPDATA%/foo"));
        assert_eq!(result, PathBuf::from("%APPDATA%/foo"));
    }
}

#[test]
fn test_expand_home_localappdata() {
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        let result = expand_home(Path::new("%LOCALAPPDATA%/foo"));
        assert_eq!(result, PathBuf::from(localappdata).join("foo"));
    } else {
        let result = expand_home(Path::new("%LOCALAPPDATA%/foo"));
        assert_eq!(result, PathBuf::from("%LOCALAPPDATA%/foo"));
    }
}

#[test]
fn test_expand_home_literal_percent_no_var() {
    let result = expand_home(Path::new("%NOVAR%/x"));
    assert_eq!(result, PathBuf::from("%NOVAR%/x"));
}

#[test]
fn test_check_readable_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("readable.txt");
    std::fs::write(&file, b"hello").unwrap();
    assert!(check_readable(&file));
}

#[test]
fn test_check_readable_nonexistent_file() {
    assert!(!check_readable(Path::new(
        "/tmp/_nonexistent_closeclaw_test_file"
    )));
}

#[test]
fn test_check_writable_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("writable.txt");
    std::fs::write(&file, b"hello").unwrap();
    assert!(check_writable(&file));
}

#[test]
fn test_check_writable_nonexistent_file() {
    assert!(!check_writable(Path::new(
        "/tmp/_nonexistent_closeclaw_test_file"
    )));
}

#[test]
fn test_check_executable_directory() {
    // Directories typically have the execute bit set on Unix
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    assert!(check_executable(dir.path()));
    #[cfg(not(unix))]
    assert!(check_executable(dir.path()));
}
