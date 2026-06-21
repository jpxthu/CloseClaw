use crate::platform::fs::normalize_path;
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
