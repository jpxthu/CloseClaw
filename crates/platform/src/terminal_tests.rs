use crate::terminal::{current_uid, detect, is_terminal, resolve_terminal_width, supports_ansi};

// ── resolve_terminal_width tests ─────────────────────────────────

#[test]
fn test_resolve_terminal_width_some_returns_width() {
    assert_eq!(resolve_terminal_width(Some(120)), 120);
    assert_eq!(resolve_terminal_width(Some(80)), 80);
    assert_eq!(resolve_terminal_width(Some(200)), 200);
}

#[test]
fn test_resolve_terminal_width_none_returns_80() {
    assert_eq!(resolve_terminal_width(None), 80);
}

#[test]
fn test_resolve_terminal_width_zero_returns_zero() {
    assert_eq!(resolve_terminal_width(Some(0)), 0);
}

// ── detect() tests ──────────────────────────────────────────────

#[test]
fn test_detect_ansi_matches_supports_ansi() {
    let info = detect();
    let expected_ansi = supports_ansi();
    assert_eq!(
        info.ansi, expected_ansi,
        "detect().ansi must equal supports_ansi()"
    );
}

#[test]
fn test_detect_deterministic() {
    let a = detect();
    let b = detect();
    assert_eq!(a.ansi, b.ansi, "detect() ANSI flag must be deterministic");
    assert_eq!(a.width, b.width, "detect() width must be deterministic");
}

#[test]
fn test_detect_width_fallback_in_non_tty() {
    // When stdin is not a TTY (piped/subagent), detect() should
    // return a sensible width.  The actual value depends on the
    // environment; in a non-TTY it should be the fallback (80).
    // In a TTY the width is the real terminal width (>0).
    let info = detect();
    if !is_terminal() {
        assert_eq!(info.width, 80, "non-TTY width must fall back to 80");
    }
    // TTY or not, width must be non-negative (usize is always >= 0).
}

// ── existing tests (preserved) ──────────────────────────────────

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
    assert!(
        !uid.contains(char::is_whitespace),
        "current_uid() must not contain whitespace: {uid}"
    );
}

#[test]
fn test_supports_ansi_returns_bool() {
    let result = supports_ansi();
    let result2 = supports_ansi();
    assert_eq!(result, result2, "supports_ansi() must be deterministic");
}

#[test]
fn test_supports_ansi_no_dumb_term() {
    let _ = supports_ansi();
}

#[test]
fn test_is_terminal_returns_bool() {
    let result = is_terminal();
    let result2 = is_terminal();
    assert_eq!(result, result2, "is_terminal() must be deterministic");
}

#[test]
fn test_is_terminal_not_tty_in_ci() {
    if !is_terminal() {
        assert!(
            !is_terminal(),
            "is_terminal() should consistently return false for piped stdin"
        );
    }
}
