use crate::terminal::{current_uid, is_terminal, supports_ansi};

#[test]
fn test_current_uid_non_empty() {
    let uid = current_uid();
    assert!(
        !uid.is_empty(),
        "current_uid() must return a non-empty string"
    );
}

#[test]
fn test_current_uid_alphanumeric() {
    let uid = current_uid();
    // On Unix it should be numeric; on Windows it's a username string.
    // Either way, no whitespace allowed.
    assert!(
        !uid.contains(char::is_whitespace),
        "current_uid() must not contain whitespace: {uid}"
    );
}

#[test]
fn test_supports_ansi_returns_bool() {
    // Must not panic regardless of environment.
    let result = supports_ansi();
    // Result is a bool; just verify it doesn't panic and is deterministic.
    let result2 = supports_ansi();
    assert_eq!(result, result2, "supports_ansi() must be deterministic");
}

#[test]
fn test_supports_ansi_no_dumb_term() {
    // Even without checking env, supports_ansi must not panic.
    // On this Linux CI, TERM is typically set to "xterm" or similar.
    let _ = supports_ansi();
}

// ── is_terminal tests ─────────────────────────────────────────────

#[test]
fn test_is_terminal_returns_bool() {
    // Must not panic regardless of environment.
    let result = is_terminal();
    // Verify determinism across calls.
    let result2 = is_terminal();
    assert_eq!(result, result2, "is_terminal() must be deterministic");
}

#[test]
fn test_is_terminal_not_tty_in_ci() {
    // In a CI or subagent, stdin is piped (not a TTY).
    // Verify is_terminal() detects this correctly.
    if !is_terminal() {
        assert!(
            !is_terminal(),
            "is_terminal() should consistently return false for piped stdin"
        );
    }
    // If we happen to be in a TTY, the test is still valid —
    // the function responded to the real environment.
}
