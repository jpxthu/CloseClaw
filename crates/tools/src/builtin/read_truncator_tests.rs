use super::read_truncator::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a string with exactly `n` lines, each of the form "line {i}\n".
fn make_lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

/// Build a string where each line is `line_size` bytes.
fn make_long_lines(count: usize, line_size: usize) -> String {
    let filler = "x".repeat(line_size);
    (0..count).map(|i| format!("{filler}{i}\n")).collect()
}

// ---------------------------------------------------------------------------
// truncate_lines — normal paths
// ---------------------------------------------------------------------------

#[test]
fn test_small_file_no_truncation() {
    let content = "line 1\nline 2\nline 3\n";
    let cfg = TruncationConfig::default();
    let r = truncate_lines(content, 1, None, &cfg);
    assert!(!r.truncated);
    assert_eq!(r.lines_read, 3);
    assert_eq!(r.total_lines, 3);
    assert!(r.trigger.is_none());
}

#[test]
fn test_offset_starts_from_correct_line() {
    let content = "a\nb\nc\nd\ne\n";
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 3, None, &cfg);
    assert!(!r.truncated);
    assert_eq!(r.lines_read, 3); // c, d, e
    assert!(r.content.starts_with("c\n"));
}

#[test]
fn test_offset_beyond_file_returns_empty() {
    let content = "a\nb\n";
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 100, None, &cfg);
    assert!(r.truncated);
    assert_eq!(r.lines_read, 0);
    assert!(r.content.is_empty());
}

#[test]
fn test_offset_zero_treated_as_one() {
    let content = "a\nb\nc\n";
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 0, None, &cfg);
    assert_eq!(r.lines_read, 3);
    assert!(r.content.starts_with("a\n"));
}

// ---------------------------------------------------------------------------
// truncate_lines — limit
// ---------------------------------------------------------------------------

#[test]
fn test_limit_restricts_output() {
    let content = "a\nb\nc\nd\ne\n";
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 1, Some(2), &cfg);
    assert!(r.truncated);
    assert_eq!(r.lines_read, 2);
    assert_eq!(r.trigger, Some(TruncationTrigger::Limit));
}

#[test]
fn test_limit_zero_returns_empty() {
    let content = "a\nb\n";
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 1, Some(0), &cfg);
    assert!(r.truncated);
    assert_eq!(r.lines_read, 0);
}

#[test]
fn test_limit_greater_than_total_returns_all() {
    let content = "a\nb\n";
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 1, Some(100), &cfg);
    assert!(!r.truncated);
    assert_eq!(r.lines_read, 2);
}

// ---------------------------------------------------------------------------
// truncate_lines — line threshold
// ---------------------------------------------------------------------------

#[test]
fn test_line_threshold_truncation() {
    let content = make_lines(2500);
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 1, None, &cfg);
    assert!(r.truncated);
    assert_eq!(r.lines_read, 2000);
    assert_eq!(r.trigger, Some(TruncationTrigger::Lines));
}

// ---------------------------------------------------------------------------
// truncate_lines — byte threshold
// ---------------------------------------------------------------------------

#[test]
fn test_byte_threshold_truncation() {
    // Each line is ~1KB + overhead. 60 lines > 50KB.
    let content = make_long_lines(60, 1024);
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 1, None, &cfg);
    assert!(r.truncated);
    assert_eq!(r.trigger, Some(TruncationTrigger::Bytes));
}

// ---------------------------------------------------------------------------
// truncate_lines — token threshold
// ---------------------------------------------------------------------------

#[test]
fn test_token_threshold_truncation() {
    // Each line ~200 bytes = ~50 tokens. 400 lines = 20000 tokens
    // but max_tokens = 51200/4 = 12800, so ~256 lines.
    let content = make_long_lines(400, 200);
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 1, None, &cfg);
    assert!(r.truncated);
    assert!(r.lines_read < 400);
}

// ---------------------------------------------------------------------------
// truncate_lines — single line exceeds byte limit
// ---------------------------------------------------------------------------

#[test]
fn test_single_long_line_triggers_special_case() {
    let long_line = "x".repeat(SINGLE_LINE_BYTE_LIMIT + 100);
    let content = format!("{long_line}\nnext line\n");
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 1, None, &cfg);
    assert!(r.truncated);
    assert_eq!(r.lines_read, 1);
    assert_eq!(r.trigger, Some(TruncationTrigger::Bytes));
    assert!(r.content.contains(&long_line));
}

// ---------------------------------------------------------------------------
// truncate_lines — combined offset + limit
// ---------------------------------------------------------------------------

#[test]
fn test_offset_and_limit_together() {
    let content = make_lines(10);
    let cfg = TruncationConfig::default();
    let r = truncate_lines(&content, 4, Some(3), &cfg);
    assert!(r.truncated);
    assert_eq!(r.lines_read, 3);
    assert!(r.content.starts_with("line 4\n"));
    assert!(r.content.contains("line 5\n"));
    assert!(r.content.contains("line 6\n"));
    assert!(!r.content.contains("line 7\n"));
}

// ---------------------------------------------------------------------------
// format_truncation_message
// ---------------------------------------------------------------------------

#[test]
fn test_format_no_truncation_returns_none() {
    let r = TruncationResult {
        content: "hello\n".into(),
        truncated: false,
        lines_read: 1,
        total_lines: 1,
        trigger: None,
    };
    assert!(format_truncation_message(&r, 1).is_none());
}

#[test]
fn test_format_lines_trigger() {
    let r = TruncationResult {
        content: String::new(),
        truncated: true,
        lines_read: 2000,
        total_lines: 5000,
        trigger: Some(TruncationTrigger::Lines),
    };
    let msg = format_truncation_message(&r, 1).unwrap();
    assert_eq!(
        msg,
        "[Showing lines 1-2000 of 5000. Use offset=2001 to continue.]"
    );
}

#[test]
fn test_format_bytes_trigger() {
    let r = TruncationResult {
        content: String::new(),
        truncated: true,
        lines_read: 150,
        total_lines: 300,
        trigger: Some(TruncationTrigger::Bytes),
    };
    let msg = format_truncation_message(&r, 1).unwrap();
    assert!(msg.contains("50KB limit"));
    assert!(msg.contains("Use offset=151 to continue."));
}

#[test]
fn test_format_limit_trigger() {
    let r = TruncationResult {
        content: String::new(),
        truncated: true,
        lines_read: 5,
        total_lines: 20,
        trigger: Some(TruncationTrigger::Limit),
    };
    let msg = format_truncation_message(&r, 10).unwrap();
    assert_eq!(msg, "[6 more lines in file. Use offset=15 to continue.]");
}

#[test]
fn test_format_offset_calculation() {
    let r = TruncationResult {
        content: String::new(),
        truncated: true,
        lines_read: 100,
        total_lines: 500,
        trigger: Some(TruncationTrigger::Lines),
    };
    let msg = format_truncation_message(&r, 50).unwrap();
    // start=50, end=149, next=150
    assert!(msg.contains("Showing lines 50-149"));
    assert!(msg.contains("Use offset=150"));
}

// ---------------------------------------------------------------------------
// Default config
// ---------------------------------------------------------------------------

#[test]
fn test_default_config_values() {
    let cfg = TruncationConfig::default();
    assert_eq!(cfg.max_lines, 2000);
    assert_eq!(cfg.max_bytes, 51_200);
    assert_eq!(cfg.max_tokens, 12_800);
}

// ---------------------------------------------------------------------------
// human_readable_bytes
// ---------------------------------------------------------------------------

#[test]
fn test_human_readable_bytes() {
    // We can't call human_readable_bytes directly since it's private,
    // but we can verify the bytes limit message format.
    let r = TruncationResult {
        content: String::new(),
        truncated: true,
        lines_read: 10,
        total_lines: 20,
        trigger: Some(TruncationTrigger::Bytes),
    };
    let msg = format_truncation_message(&r, 1).unwrap();
    // 51200 / 1024 = 50, so "50KB"
    assert!(msg.contains("50KB"));
}
