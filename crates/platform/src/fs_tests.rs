use crate::fs::{
    check_executable, check_readable, check_writable, expand_home, normalize_path, set_executable,
};
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

/// Absolute path (no tilde prefix) should pass through unchanged.
#[test]
fn test_expand_home_absolute_path() {
    let result = expand_home(Path::new("/usr/local/bin"));
    assert_eq!(result, PathBuf::from("/usr/local/bin"));
}

/// `~` with trailing slash but no further path — expand to home dir.
#[test]
fn test_expand_home_tilde_slash_only() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/test".to_string());
    let result = expand_home(Path::new("~/"));
    assert_eq!(result, PathBuf::from(home));
}

/// Relative path without tilde should pass through unchanged.
#[test]
fn test_expand_home_relative_path() {
    let result = expand_home(Path::new("relative/path"));
    assert_eq!(result, PathBuf::from("relative/path"));
}

/// Tilde in the middle of a path (not at start) should pass through.
#[test]
fn test_expand_home_tilde_not_at_start() {
    let result = expand_home(Path::new("foo/~bar"));
    assert_eq!(result, PathBuf::from("foo/~bar"));
}

#[test]
fn test_expand_home_percent_var_passthrough() {
    // %VAR% syntax is not expanded; paths are returned unchanged.
    let result = expand_home(Path::new("%APPDATA%/foo"));
    assert_eq!(result, PathBuf::from("%APPDATA%/foo"));
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
    assert!(check_executable(dir.path()));
}

/// Relative path without tilde should not be modified by normalize_path.
#[test]
fn test_normalize_path_relative() {
    let path = Path::new("relative/path/to/file");
    let normalized = normalize_path(path);
    assert_eq!(normalized, PathBuf::from("relative/path/to/file"));
}

/// Home dir (~) is not expanded by normalize_path — only backslashes.
#[test]
fn test_normalize_path_home_dir_not_expanded() {
    let path = Path::new(r"~\.closeclaw\config");
    let normalized = normalize_path(path);
    assert_eq!(normalized, PathBuf::from("~/.closeclaw/config"));
}

#[test]
fn test_set_executable_toggle() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("script.sh");
    std::fs::write(&file, b"#!/bin/sh\necho hi").unwrap();

    // Remove execute bit
    set_executable(&file, false).unwrap();
    assert!(!check_executable(&file));

    // Set execute bit
    set_executable(&file, true).unwrap();
    assert!(check_executable(&file));
}
