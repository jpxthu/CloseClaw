use crate::platform::config::config_dir;

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
        path_str.ends_with("/.closeclaw"),
        "Unix config_dir must end with /.closeclaw: {path_str}"
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
fn test_config_dir_not_empty_component() {
    let dir = config_dir().unwrap();
    let components: Vec<_> = dir.components().collect();
    assert!(
        components.len() >= 2,
        "config_dir must have at least 2 path components"
    );
}
