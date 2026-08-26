use crate::terminal::{
    current_uid, detect, detect_with_size, is_terminal, resolve_terminal_width, supports_ansi,
    supports_ansi_inner, TerminalInfo,
};

// ── TerminalInfo derive trait tests ───────────────────────────────

#[test]
fn test_terminal_info_debug() {
    let info = TerminalInfo {
        ansi: true,
        width: 120,
    };
    let dbg = format!("{:?}", info);
    assert!(dbg.contains("ansi"), "Debug output must include field name");
    assert!(dbg.contains("120"), "Debug output must include width value");
}

#[test]
fn test_terminal_info_clone() {
    let info = TerminalInfo {
        ansi: false,
        width: 80,
    };
    let cloned = info.clone();
    assert_eq!(info, cloned);
}

#[test]
fn test_terminal_info_copy() {
    let info = TerminalInfo {
        ansi: true,
        width: 200,
    };
    let copied = info; // Copy, not move
    assert_eq!(info, copied);
    // Original is still usable (Copy trait).
    assert_eq!(info.width, 200);
}

#[test]
fn test_terminal_info_eq_and_ne() {
    let a = TerminalInfo {
        ansi: true,
        width: 100,
    };
    let b = TerminalInfo {
        ansi: true,
        width: 100,
    };
    let c = TerminalInfo {
        ansi: false,
        width: 100,
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

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

// ── detect_with_size: injectable tests ───────────────────────────

#[test]
fn test_detect_with_size_some_returns_width() {
    let info = detect_with_size(|| Some(120));
    assert_eq!(info.width, 120, "Some(120) should yield width 120");
}

#[test]
fn test_detect_with_size_none_returns_80() {
    let info = detect_with_size(|| None);
    assert_eq!(info.width, 80, "None should fall back to 80");
}

#[test]
fn test_detect_with_size_ansi_matches_supports_ansi() {
    let info = detect_with_size(|| Some(80));
    assert_eq!(
        info.ansi,
        supports_ansi(),
        "detect_with_size ANSI must equal supports_ansi()"
    );
}

#[test]
fn test_detect_with_size_is_deterministic() {
    let a = detect_with_size(|| Some(150));
    let b = detect_with_size(|| Some(150));
    assert_eq!(a, b, "same input must produce equal TerminalInfo");
}

// ── detect(): public API tests (environment-dependent) ───────────

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

/// Empty TERM string should not match any ANSI pattern.
#[test]
fn test_supports_ansi_inner_empty_term() {
    assert!(
        !supports_ansi_inner(Some("")),
        "empty TERM should not support ANSI"
    );
}

/// "xterm-256color" contains "xterm" → should be recognized.
#[test]
fn test_supports_ansi_inner_xterm_256color() {
    assert!(
        supports_ansi_inner(Some("xterm-256color")),
        "xterm-256color should support ANSI"
    );
}

/// "dumb" TERM should not be recognized as ANSI-capable.
#[test]
fn test_supports_ansi_inner_dumb_term() {
    assert!(
        !supports_ansi_inner(Some("dumb")),
        "dumb TERM should not support ANSI"
    );
}

/// Unset TERM should not support ANSI.
#[test]
fn test_supports_ansi_inner_unset_term() {
    assert!(
        !supports_ansi_inner(None),
        "unset TERM should not support ANSI"
    );
}

/// Case-insensitive: "XTERM" should be recognized.
#[test]
fn test_supports_ansi_inner_case_insensitive() {
    assert!(
        supports_ansi_inner(Some("XTERM")),
        "uppercase XTERM should support ANSI"
    );
    assert!(
        supports_ansi_inner(Some("Screen")),
        "mixed-case Screen should support ANSI"
    );
}

/// "screen-256color" contains "screen" → should be recognized.
#[test]
fn test_supports_ansi_inner_screen_color() {
    assert!(
        supports_ansi_inner(Some("screen-256color")),
        "screen-256color should support ANSI"
    );
}

/// "vt100" should be recognized.
#[test]
fn test_supports_ansi_inner_vt100() {
    assert!(
        supports_ansi_inner(Some("vt100")),
        "vt100 should support ANSI"
    );
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
