use crate::fs::{
    check_executable, check_readable, check_writable, expand_env, expand_home, expand_path,
    normalize_path, set_executable, to_platform_path,
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

// --- to_platform_path tests ---

#[test]
fn test_to_platform_path_identity_unix() {
    let path = Path::new("/usr/local/bin");
    let result = to_platform_path(path);
    assert_eq!(result, PathBuf::from("/usr/local/bin"));
}

#[test]
fn test_to_platform_path_backslashes() {
    let path = Path::new(r"C:\Users\test\file.txt");
    let result = to_platform_path(path);
    assert_eq!(result, PathBuf::from("C:/Users/test/file.txt"));
}

#[test]
fn test_to_platform_path_mixed_separators() {
    let path = Path::new(r"C:\Users/test\another/file");
    let result = to_platform_path(path);
    assert_eq!(result, PathBuf::from("C:/Users/test/another/file"));
}

#[test]
fn test_to_platform_path_empty() {
    let path = Path::new("");
    let result = to_platform_path(path);
    assert_eq!(result, PathBuf::from(""));
}

#[test]
fn test_to_platform_path_trailing_separator() {
    let path = Path::new(r"C:\Users\test\");
    let result = to_platform_path(path);
    assert_eq!(result, PathBuf::from("C:/Users/test/"));
}

/// On `/`-separator platforms, `to_platform_path` and `normalize_path` produce
/// identical output — this round-trip property must hold.
#[test]
fn test_to_platform_path_roundtrip_with_normalize() {
    let inputs = [
        r"C:\Users\test",
        "/usr/local/bin",
        "relative/path",
        r"mixed\\path/here",
    ];
    for input in &inputs {
        let p = Path::new(input);
        assert_eq!(to_platform_path(p), normalize_path(p));
    }
}

// --- expand_home tests ---

#[test]
fn test_expand_home_tilde() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/test"));
    let result = expand_home(Path::new("~/foo"));
    assert_eq!(result, home.join("foo"));
}

#[test]
fn test_expand_home_bare_tilde() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/test"));
    let result = expand_home(Path::new("~"));
    assert_eq!(result, home);
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
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/test"));
    let result = expand_home(Path::new("~/"));
    assert_eq!(result, home);
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

/// `~otheruser` is not a home shorthand — preserved as literal.
#[test]
fn test_expand_home_other_user_not_expanded() {
    let result = expand_home(Path::new("~otheruser/foo"));
    assert_eq!(result, PathBuf::from("~otheruser/foo"));
}

/// `~alice/data` — non-current-user prefix preserved.
#[test]
fn test_expand_home_other_user() {
    let result = expand_home(Path::new("~alice/data"));
    assert_eq!(result, PathBuf::from("~alice/data"));
}

// --- expand_env tests ---
// Tests focus on undefined variables (preservation) and edge-case
// parsing. Defined-variable expansion is validated via integration tests.

/// Undefined variable is preserved as literal text.
#[test]
fn test_expand_env_undefined_var_preserved() {
    let result = expand_env(Path::new("$NO_SUCH_VAR_xyz/foo"));
    assert_eq!(result, PathBuf::from("$NO_SUCH_VAR_xyz/foo"));
}

/// Undefined variable with brace syntax preserved.
#[test]
fn test_expand_env_undefined_brace_var_preserved() {
    let result = expand_env(Path::new("${NO_SUCH_VAR_xyz}/bar"));
    assert_eq!(result, PathBuf::from("${NO_SUCH_VAR_xyz}/bar"));
}

/// A bare `$` at end of path — no valid var name follows, preserved.
#[test]
fn test_expand_env_dollar_bare_at_end() {
    let result = expand_env(Path::new("foo/bar$"));
    assert_eq!(result, PathBuf::from("foo/bar$"));
}

/// `${}` — empty var name, preserved as literal.
#[test]
fn test_expand_env_empty_braces() {
    let result = expand_env(Path::new("foo/${}/bar"));
    assert_eq!(result, PathBuf::from("foo/${}/bar"));
}

/// No `$` in path — passthrough without allocation.
#[test]
fn test_expand_env_no_dollar_passthrough() {
    let result = expand_env(Path::new("/usr/local/bin"));
    assert_eq!(result, PathBuf::from("/usr/local/bin"));
}

/// Multiple undefined variables in a row — each preserved.
#[test]
fn test_expand_env_multiple_undefined_consecutive() {
    let result = expand_env(Path::new("$UNDEF_A$UNDEF_B"));
    assert_eq!(result, PathBuf::from("$UNDEF_A$UNDEF_B"));
}

/// `$` followed by non-identifier char (digit) — not a var reference.
#[test]
fn test_expand_env_dollar_digit() {
    let result = expand_env(Path::new("foo/$1bar"));
    assert_eq!(result, PathBuf::from("foo/$1bar"));
}

/// `$` in the middle of a word (surrounded by alphanumeric) — not matched
/// because regex requires `$` at a word boundary (preceded by non-id char).
#[test]
fn test_expand_env_dollar_in_middle_of_word() {
    let result = expand_env(Path::new("foo/bar$baz/qux"));
    // $baz is preceded by $, which is not an identifier char, so it matches
    // and $baz is preserved as undefined.
    assert_eq!(result, PathBuf::from("foo/bar$baz/qux"));
}

/// Path with `$` only in a segment that is not a var name.
#[test]
fn test_expand_env_dollar_not_var_name() {
    let result = expand_env(Path::new("foo/$/bar"));
    assert_eq!(result, PathBuf::from("foo/$/bar"));
}

/// Mixed: some undefined vars and some literal `$` chars.
#[test]
fn test_expand_env_mixed_undefined_and_literal() {
    let result = expand_env(Path::new("$UNDEF1/$UNDEF2/$"));
    assert_eq!(result, PathBuf::from("$UNDEF1/$UNDEF2/$"));
}

/// Path with `${VAR}` where VAR contains underscore and digits.
#[test]
fn test_expand_env_underscore_and_digits() {
    let result = expand_env(Path::new("${MY_VAR_123}/sub"));
    assert_eq!(result, PathBuf::from("${MY_VAR_123}/sub"));
}

// --- expand_path tests ---

/// expand_path chains expand_home + expand_env + normalize_path.
#[test]
fn test_expand_path_tilde() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/test"));
    let result = expand_path(Path::new("~/foo"));
    assert_eq!(result, home.join("foo"));
}

/// expand_path with bare `~` expands to home.
#[test]
fn test_expand_path_bare_tilde() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/test"));
    let result = expand_path(Path::new("~"));
    assert_eq!(result, home);
}

/// Absolute path without tilde — passthrough.
#[test]
fn test_expand_path_absolute_no_tilde() {
    let result = expand_path(Path::new("/usr/local/bin"));
    assert_eq!(result, PathBuf::from("/usr/local/bin"));
}

/// expand_path with undefined env var preserves the literal.
#[test]
fn test_expand_path_with_undefined_env() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/test"));
    let result = expand_path(Path::new("~/$_UNDEFINED_XYZ_/sub"));
    assert_eq!(result, home.join("$_UNDEFINED_XYZ_/sub"));
}

/// expand_path normalizes backslashes.
#[test]
fn test_expand_path_normalizes_backslashes() {
    let result = expand_path(Path::new(r"C:\Users\test"));
    assert_eq!(result, PathBuf::from("C:/Users/test"));
}

// --- check_ / set_executable tests ---

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
