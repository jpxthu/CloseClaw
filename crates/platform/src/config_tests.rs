use crate::config::{config_dir, root_dir};

#[test]
fn test_root_dir_returns_valid_path() {
    let dir = root_dir().unwrap();
    assert!(dir.is_absolute(), "root_dir must return an absolute path");

    let path_str = dir.to_string_lossy();
    assert!(
        path_str.contains("closeclaw"),
        "root_dir must contain 'closeclaw': {path_str}"
    );
}

#[cfg(unix)]
#[test]
fn test_root_dir_unix_format() {
    let dir = root_dir().unwrap();
    let path_str = dir.to_string_lossy();
    assert!(
        path_str.ends_with("/.closeclaw"),
        "Unix root_dir must end with /.closeclaw: {path_str}"
    );
}

#[cfg(unix)]
#[test]
fn test_root_dir_starts_with_home() {
    let dir = root_dir().unwrap();
    let home = std::env::var("HOME").unwrap();
    let path_str = dir.to_string_lossy();
    assert!(
        path_str.starts_with(&home),
        "root_dir must start with HOME ({home}): {path_str}"
    );
}

#[test]
fn test_root_dir_not_empty_component() {
    let dir = root_dir().unwrap();
    let components: Vec<_> = dir.components().collect();
    assert!(
        components.len() >= 2,
        "root_dir must have at least 2 path components"
    );
}

#[test]
fn test_config_dir_returns_valid_path() {
    let dir = config_dir().unwrap();
    assert!(dir.is_absolute(), "config_dir must return an absolute path");

    let path_str = dir.to_string_lossy();
    assert!(
        path_str.contains("closeclaw"),
        "config_dir must contain 'closeclaw': {path_str}"
    );
}

#[cfg(unix)]
#[test]
fn test_config_dir_unix_format() {
    let dir = config_dir().unwrap();
    let path_str = dir.to_string_lossy();
    assert!(
        path_str.ends_with("/.closeclaw/config"),
        "Unix config_dir must end with /.closeclaw/config: {path_str}"
    );
}

#[cfg(unix)]
#[test]
fn test_config_dir_starts_with_home() {
    let dir = config_dir().unwrap();
    let home = std::env::var("HOME").unwrap();
    let path_str = dir.to_string_lossy();
    assert!(
        path_str.starts_with(&home),
        "config_dir must start with HOME ({home}): {path_str}"
    );
}

#[test]
fn test_config_dir_is_child_of_root_dir() {
    let root = root_dir().unwrap();
    let config = config_dir().unwrap();
    assert_eq!(
        config.parent(),
        Some(root.as_path()),
        "config_dir must be a direct child of root_dir"
    );
}
