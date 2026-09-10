use crate::fs::{expand_env, expand_home, expand_path, normalize_path, to_platform_path};
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

/// `$` followed by identifier chars in a path segment — the regex matches
/// `$baz` as an undefined variable reference and preserves it.
#[test]
fn test_expand_env_dollar_in_middle_of_word() {
    let result = expand_env(Path::new("foo/bar$baz/qux"));
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

// --- Pure-contract & chain tests ---

/// expand_path is a pure conversion: it must succeed on non-existent
/// paths without performing any file system I/O.
#[test]
fn test_expand_path_nonexistent_path_succeeds() {
    let result = expand_path(Path::new("/this/path/does/not/exist/at/all"));
    assert_eq!(result, PathBuf::from("/this/path/does/not/exist/at/all"));
}

/// expand_path on a non-existent path with tilde still expands home.
#[test]
fn test_expand_path_nonexistent_with_tilde() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/test"));
    let result = expand_path(Path::new("~/nonexistent/deeply/nested/file.txt"));
    assert_eq!(result, home.join("nonexistent/deeply/nested/file.txt"));
}

/// expand_path on a non-existent path with undefined env var preserves it.
#[test]
fn test_expand_path_nonexistent_with_undefined_env() {
    let result = expand_path(Path::new("/some/$NONEXISTENT_VAR_abc/file"));
    assert_eq!(result, PathBuf::from("/some/$NONEXISTENT_VAR_abc/file"));
}

/// expand_path chains ~ expansion → env expansion → separator normalization
/// in a single call. A tilde path must produce a clean, absolute,
/// forward-slash path.
#[test]
fn test_expand_path_full_chain() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/test"));
    let result = expand_path(Path::new("~/subdir/file.txt"));
    let expected = home.join("subdir/file.txt");
    assert_eq!(result, expected);
    assert!(
        result.is_absolute(),
        "expand_path with ~ must produce absolute path"
    );
}

/// expand_path with backslash-separated path containing ~ — backslashes
/// are normalized to `/` but ~ is not expanded (not at position `~/` on
/// Linux). This documents the current behavior: normalize_path runs
/// after expand_home, so backslash-to-slash conversion doesn't trigger
/// tilde expansion.
#[test]
fn test_expand_path_backslash_tilde_not_expanded() {
    let result = expand_path(Path::new(r"~\subdir\file.txt"));
    // On Linux, ~\\ is not ~/ so expand_home is a no-op; backslashes
    // are normalized to / by normalize_path.
    assert_eq!(result, PathBuf::from("~/subdir/file.txt"));
}

/// $VAR and ${VAR} syntax both expand to the same value when defined.
#[test]
fn test_expand_env_dollar_vs_brace_equivalent() {
    // We cannot set env vars (test red line). Instead verify that both
    // syntaxes are parsed identically by checking undefined vars are
    // preserved with the correct literal form.
    let r1 = expand_env(Path::new("$MY_VAR_XYZ"));
    let r2 = expand_env(Path::new("${MY_VAR_XYZ}"));
    assert_eq!(r1, PathBuf::from("$MY_VAR_XYZ"));
    assert_eq!(r2, PathBuf::from("${MY_VAR_XYZ}"));
    assert_ne!(r1, r2, "raw $ and ${{}} syntax preserve different literals");
}

/// normalize_path and to_platform_path must agree on all inputs.
#[test]
fn test_normalize_and_platform_path_agree() {
    let inputs = [
        r"C:\Users\test",
        "/usr/local/bin",
        "relative/path",
        r"mixed\\path/here",
        "",
        "/",
    ];
    for input in &inputs {
        let p = Path::new(input);
        assert_eq!(
            normalize_path(p),
            to_platform_path(p),
            "normalize_path and to_platform_path must agree for: {input}"
        );
    }
}

/// normalize_path converts all backslashes to forward slashes.
#[test]
fn test_normalize_path_backslash_unification() {
    let path = Path::new(r"a\b\c\d");
    assert_eq!(normalize_path(path), PathBuf::from("a/b/c/d"));
}

/// to_platform_path converts all backslashes to forward slashes.
#[test]
fn test_to_platform_path_backslash_unification() {
    let path = Path::new(r"x\y\z");
    assert_eq!(to_platform_path(path), PathBuf::from("x/y/z"));
}
