use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_canonicalize_existing_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.txt");
    fs::write(&file, "hello").unwrap();

    let result = canonicalize_or_clone(&file);
    assert_eq!(result, fs::canonicalize(&file).unwrap());
}

#[test]
fn test_canonicalize_nonexistent_falls_back() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("does_not_exist.txt");

    let result = canonicalize_or_clone(&file);
    assert_eq!(result, file);
}

#[test]
fn test_canonicalize_dot_segments() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.txt");
    fs::write(&file, "hello").unwrap();

    let dotted = dir.path().join("./test.txt");
    let result = canonicalize_or_clone(&dotted);
    assert_eq!(result, fs::canonicalize(&file).unwrap());
}

#[test]
fn test_canonicalize_symlink() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.txt");
    let link = dir.path().join("link.txt");
    fs::write(&real, "hello").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let result_from_link = canonicalize_or_clone(&link);
    let result_from_real = canonicalize_or_clone(&real);
    assert_eq!(result_from_link, result_from_real);
}
