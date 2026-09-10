//! Integration tests for expand_path with environment variable expansion.
//!
//! Tests the full pipeline: expand_home → expand_env → normalize_path.
//! Uses $HOME (always set) to validate that defined variable expansion
//! works through the expand_path entry point.

use closeclaw_platform::fs::{expand_env, expand_home, expand_path};
use std::path::{Path, PathBuf};

/// expand_path chains ~ expansion + env expansion + normalization.
/// This is the core design-doc requirement: one entry point handles all.
#[test]
fn expand_path_full_chain_tilde_and_home_env() {
    let home = dirs::home_dir().unwrap();
    // ~/data → home/data (tilde expansion only, no env vars in this path)
    let result = expand_path(Path::new("~/data"));
    assert_eq!(result, home.join("data"));
}

/// expand_path with $HOME in the path — env expansion after tilde expansion.
#[test]
fn expand_path_env_expansion_with_home_var() {
    let home = dirs::home_dir().unwrap();
    // $HOME/test → home/test
    let result = expand_path(Path::new("$HOME/test"));
    assert_eq!(result, home.join("test"));
}

/// expand_path with ${HOME} brace syntax — same result.
#[test]
fn expand_path_brace_env_expansion() {
    let home = dirs::home_dir().unwrap();
    let result = expand_path(Path::new("${HOME}/test"));
    assert_eq!(result, home.join("test"));
}

/// expand_env with $HOME — direct env expansion.
#[test]
fn expand_env_with_home_var() {
    let home = dirs::home_dir().unwrap();
    let result = expand_env(Path::new("$HOME/sub"));
    assert_eq!(result, home.join("sub"));
}

/// expand_env with undefined var — preserved as literal.
#[test]
fn expand_env_undefined_preserved() {
    let result = expand_env(Path::new("$UNDEFINED_XYZ_12345/sub"));
    assert_eq!(result, PathBuf::from("$UNDEFINED_XYZ_12345/sub"));
}

/// expand_home bare tilde → home dir.
#[test]
fn expand_home_bare() {
    let home = dirs::home_dir().unwrap();
    assert_eq!(expand_home(Path::new("~")), home);
}

/// expand_home ~otheruser → preserved.
#[test]
fn expand_home_other_user_preserved() {
    assert_eq!(
        expand_home(Path::new("~otheruser/data")),
        PathBuf::from("~otheruser/data")
    );
}

/// expand_home ~/ → home dir.
#[test]
fn expand_home_slash_only() {
    let home = dirs::home_dir().unwrap();
    assert_eq!(expand_home(Path::new("~/")), home);
}
