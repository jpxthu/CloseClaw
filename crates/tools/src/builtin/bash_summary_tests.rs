//! Unit tests for `bash_kill::truncate_summary` (issue #3529).
//!
//! Coverage dimensions:
//! - Short command (len ≤ 200) passes through unchanged, no ellipsis.
//! - Boundary: exactly 200 bytes unchanged; 201 ASCII bytes cut to
//!   200 bytes + `"..."`.
//! - Multi-byte boundary: CJK / emoji crossing the 200-byte line snaps
//!   the cut back to a character boundary — result stays valid UTF-8,
//!   no character is cut in half, total ≤ 200 + 3 bytes, no panic.
//! - All-ASCII long command and mixed-script long command covered.

use crate::builtin::bash_kill::truncate_summary;

/// Short command (len ≤ 200): returned as-is, no ellipsis appended.
#[test]
fn test_truncate_summary_short_command_unchanged() {
    let cmd = "echo hello world";
    assert_eq!(truncate_summary(cmd), cmd);
}

/// Exactly 200 bytes: still within the limit, returned as-is.
#[test]
fn test_truncate_summary_exactly_200_bytes_unchanged() {
    let cmd = "b".repeat(200);
    assert_eq!(truncate_summary(&cmd), cmd);
}

/// 201 ASCII bytes: truncated to 200 bytes + "...".
#[test]
fn test_truncate_summary_201_ascii_bytes_truncated_with_ellipsis() {
    let cmd = "a".repeat(201);
    let summary = truncate_summary(&cmd);
    assert_eq!(summary, format!("{}...", "a".repeat(200)));
    assert_eq!(summary.len(), 203);
}

/// Long all-ASCII command: cut at byte 200 with "..." appended.
#[test]
fn test_truncate_summary_long_ascii_command() {
    let cmd = format!("echo {}", "a".repeat(196)); // 5 + 196 = 201 bytes
    let summary = truncate_summary(&cmd);
    assert_eq!(summary, format!("echo {}...", "a".repeat(195)));
    assert_eq!(summary.len(), 203);
}

/// CJK command crossing the 200-byte boundary: byte 200 lands exactly
/// on the end of the 65th CJK char, so the cut keeps 65 whole chars.
#[test]
fn test_truncate_summary_cjk_boundary_on_char_end() {
    let cmd = format!("echo {}", "你".repeat(66)); // 5 + 198 = 203 bytes
    let summary = truncate_summary(&cmd);
    assert_eq!(summary, format!("echo {}...", "你".repeat(65)));
    assert_eq!(summary.len(), 203);
}

/// Byte 200 falls inside a multi-byte CJK char: the truncation point
/// snaps back to the previous character boundary (no half character,
/// no panic — a naive `&cmd[..200]` would panic here).
#[test]
fn test_truncate_summary_multibyte_boundary_snaps_back() {
    let cmd = format!("{}你", "x".repeat(198)); // 198 + 3 = 201 bytes
    let summary = truncate_summary(&cmd);
    assert_eq!(summary, format!("{}...", "x".repeat(198)));
    assert_eq!(summary.len(), 201);
}

/// Emoji (4-byte) command crossing the boundary: cut snaps back to the
/// last whole emoji before byte 200.
#[test]
fn test_truncate_summary_emoji_boundary_snaps_back() {
    let cmd = format!("git commit -m '{}'", "🚀".repeat(50)); // 15 + 200 + 1 = 216 bytes
    let summary = truncate_summary(&cmd);
    assert_eq!(summary, format!("git commit -m '{}...", "🚀".repeat(46)));
    assert_eq!(summary.len(), 202);
}

/// Mixed-script long command (ASCII + CJK + emoji): result stays valid
/// UTF-8, no character is cut in half, total ≤ 200 + 3 bytes.
#[test]
fn test_truncate_summary_mixed_script_long_command() {
    let head = "echo hello 世界 "; // 11 + 6 + 1 = 18 bytes
    let cmd = format!("{}{}", head, "🚀".repeat(46)); // 18 + 184 = 202 bytes
    let summary = truncate_summary(&cmd);
    assert!(summary.ends_with("..."));
    assert!(summary.len() <= 203);
    let prefix_len = summary.len() - 3;
    assert!(
        cmd.is_char_boundary(prefix_len),
        "cut snapped to char boundary"
    );
    assert_eq!(&summary[..prefix_len], &cmd[..prefix_len]);
    assert_eq!(summary, format!("{}{}", &cmd[..prefix_len], "..."));
}
