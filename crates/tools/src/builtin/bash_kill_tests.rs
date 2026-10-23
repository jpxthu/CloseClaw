use super::bash_kill::truncate_summary;
use super::bash_kill::BackgroundKillHandle;
use super::bash_test_support::FakeTaskManager;
use closeclaw_common::tool_session::KillHandle;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const MAX_SUMMARY_LEN: usize = 200;

/// `"a".repeat(n)`.
fn ascii(n: usize) -> String {
    "a".repeat(n)
}

/// Split a truncated summary into `(head, marker_present)`.
///
/// Panics when the summary does not end with the `"..."` marker.
fn split_summary(summary: &str) -> &str {
    summary
        .strip_suffix("...")
        .expect("truncated summary must end with \"...\"")
}

// ---------------------------------------------------------------------------
// Normal path — inputs of <= 200 bytes are returned unchanged
// ---------------------------------------------------------------------------

#[test]
fn test_empty_command_unchanged() {
    let out = truncate_summary("");
    assert_eq!(out, "");
    assert!(!out.contains("..."));
}

#[test]
fn test_short_command_unchanged() {
    let cmd = format!("ls -la {}", std::env::temp_dir().to_str().unwrap());
    let out = truncate_summary(&cmd);
    assert_eq!(out, cmd);
    assert!(!out.contains("..."));
}

#[test]
fn test_exactly_200_ascii_bytes_unchanged() {
    let cmd = ascii(200);
    assert_eq!(cmd.len(), MAX_SUMMARY_LEN);
    let out = truncate_summary(&cmd);
    assert_eq!(out, cmd);
    assert!(!out.contains("..."));
}

#[test]
fn test_exactly_200_multibyte_bytes_unchanged() {
    // 66 * 3 + 2 = 200 bytes, ending exactly on a char boundary.
    let cmd = format!("{}ab", "中".repeat(66));
    assert_eq!(cmd.len(), MAX_SUMMARY_LEN);
    let out = truncate_summary(&cmd);
    assert_eq!(out, cmd);
    assert!(!out.contains("..."));
}

// ---------------------------------------------------------------------------
// Boundary values — 201 ASCII bytes, byte 200 on a char boundary
// ---------------------------------------------------------------------------

#[test]
fn test_201_ascii_bytes_cut_at_200() {
    let cmd = ascii(201);
    let out = truncate_summary(&cmd);
    assert_eq!(out, format!("{}...", ascii(200)));
    assert_eq!(split_summary(&out).len(), 200);
}

#[test]
fn test_byte_200_exact_boundary_cuts_at_200() {
    // Byte 200 is the end of the last ASCII byte -> cut exactly at 200.
    let cmd = ascii(200);
    assert!(cmd.is_char_boundary(200));
    let out = truncate_summary(&format!("{}x", cmd));
    assert_eq!(out, format!("{}...", ascii(200)));
}

// ---------------------------------------------------------------------------
// Multibyte correctness — never split a character across byte 200
// ---------------------------------------------------------------------------

#[test]
fn test_chinese_3byte_char_inside_byte_200_cuts_before() {
    // First 中 occupies bytes [198, 201): byte 200 is its last byte, i.e.
    // inside the character -> the cut falls back to the boundary at 198.
    let cmd = format!("{}{}", ascii(198), "中".repeat(20));
    assert!(cmd.len() > MAX_SUMMARY_LEN);
    assert!(!cmd.is_char_boundary(200));
    let out = truncate_summary(&cmd);
    let head = split_summary(&out);
    assert_eq!(head, ascii(198));
    assert_eq!(head.len(), 198);
    assert!(cmd.is_char_boundary(head.len()));
}

#[test]
fn test_chinese_3byte_char_aligned_at_byte_200_cuts_at_200() {
    // First 中 occupies bytes [197, 200): byte 200 is exactly the boundary
    // after it -> the cut stays at 200.
    let cmd = format!("{}{}", ascii(197), "中".repeat(20));
    assert!(cmd.len() > MAX_SUMMARY_LEN);
    assert!(cmd.is_char_boundary(200));
    let out = truncate_summary(&cmd);
    let head = split_summary(&out);
    assert_eq!(head, format!("{}中", ascii(197)));
    assert_eq!(head.len(), 200);
    assert!(cmd.is_char_boundary(head.len()));
}

#[test]
fn test_emoji_4byte_char_inside_byte_200_cuts_before() {
    // First 😀 occupies bytes [197, 201): byte 200 is inside it -> the cut
    // falls back to the boundary at 197.
    let cmd = format!("{}{}", ascii(197), "😀".repeat(20));
    assert!(cmd.len() > MAX_SUMMARY_LEN);
    assert!(!cmd.is_char_boundary(200));
    let out = truncate_summary(&cmd);
    let head = split_summary(&out);
    assert_eq!(head, ascii(197));
    assert_eq!(head.len(), 197);
    assert!(cmd.is_char_boundary(head.len()));
}

#[test]
fn test_emoji_4byte_char_aligned_at_byte_200_cuts_at_200() {
    // First 😀 occupies bytes [196, 200): byte 200 is exactly the boundary
    // after it -> the cut stays at 200.
    let cmd = format!("{}{}", ascii(196), "😀".repeat(20));
    assert!(cmd.len() > MAX_SUMMARY_LEN);
    assert!(cmd.is_char_boundary(200));
    let out = truncate_summary(&cmd);
    let head = split_summary(&out);
    assert_eq!(head, format!("{}😀", ascii(196)));
    assert_eq!(head.len(), 200);
    assert!(cmd.is_char_boundary(head.len()));
}

// ---------------------------------------------------------------------------
// Equivalence with str::floor_char_boundary(200) — explicit expected bounds
// ---------------------------------------------------------------------------

#[test]
fn test_truncation_matches_expected_floor_char_boundary() {
    // (input, expected floor_char_boundary(200) byte offset)
    let cases: Vec<(String, usize)> = vec![
        // index 200 is already a char boundary -> cut at 200
        (format!("{}x", ascii(200)), 200),
        (ascii(201), 200),
        (ascii(1000), 200),
        (format!("{}中中中", ascii(197)), 200),
        (format!("{}😀😀😀", ascii(196)), 200),
        // index 200 falls inside a multi-byte char -> previous boundary
        (format!("{}{}", ascii(199), "中".repeat(5)), 199),
        (format!("{}{}", ascii(198), "中".repeat(5)), 198),
        (format!("{}{}", ascii(197), "😀".repeat(5)), 197),
        // mixed multibyte: 中(3) then 😀(4); boundary 199, 200 is mid-emoji
        (format!("{}中{}", ascii(100), "😀".repeat(30)), 199),
    ];

    for (cmd, expected_end) in cases {
        assert!(cmd.len() > MAX_SUMMARY_LEN, "case must exceed 200 bytes");
        assert!(
            cmd.is_char_boundary(expected_end),
            "expected floor must be a char boundary: {expected_end}"
        );
        assert!(
            expected_end <= MAX_SUMMARY_LEN,
            "expected floor must be <= 200: {expected_end}"
        );
        let out = truncate_summary(&cmd);
        let head = split_summary(&out);
        assert_eq!(
            head.len(),
            expected_end,
            "unexpected cut offset for input of {} bytes",
            cmd.len()
        );
        assert_eq!(head, &cmd[..expected_end]);
        assert_eq!(out, format!("{}...", &cmd[..expected_end]));
    }
}

// ---------------------------------------------------------------------------
// BackgroundKillHandle::kill — error path (no tokio runtime context)
// ---------------------------------------------------------------------------

#[test]
fn test_kill_without_runtime_returns_err_with_missing_runtime_message() {
    // Contract from the `kill()` doc comment: with no current tokio
    // runtime there is nothing to `block_on`, so the call fails fast
    // with a "called outside a tokio runtime" error. The Err branch
    // returns before touching the manager, so a bare fake plus an
    // arbitrary task id is a sufficient construction.
    let (_fake, bg_manager) = FakeTaskManager::pair();
    let handle = BackgroundKillHandle {
        bg_manager,
        task_id: "no-such-task".to_string(),
    };
    let err = handle
        .kill()
        .expect_err("kill must fail outside a tokio runtime");
    assert!(
        err.to_string().contains("outside a tokio runtime"),
        "unexpected error message: {err}"
    );
}
