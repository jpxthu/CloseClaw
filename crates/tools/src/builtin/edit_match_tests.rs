//! Tests for the edit-match engine.

use super::{match_and_apply, EditError, EditOp};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn edit(old: &str, new: &str) -> EditOp {
    EditOp {
        old_text: old.to_string(),
        new_text: new.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Normal paths
// ---------------------------------------------------------------------------

#[test]
fn single_exact_match_and_replace() {
    let result = match_and_apply("hello world", &[edit("world", "rust")], false).unwrap();
    assert_eq!(result, "hello rust");
}

#[test]
fn multiple_edits_reverse_order() {
    // Two adjacent edits: "aa" → "bb" and "cc" → "dd".
    // Non-incremental matching ensures both match the original content.
    let result = match_and_apply("aa cc", &[edit("aa", "bb"), edit("cc", "dd")], false).unwrap();
    assert_eq!(result, "bb dd");
}

#[test]
fn replace_all_multiple_matches() {
    let result = match_and_apply("aaa aaa aaa", &[edit("aaa", "bbb")], true).unwrap();
    assert_eq!(result, "bbb bbb bbb");
}

#[test]
fn old_text_equals_new_text() {
    let result = match_and_apply("unchanged", &[edit("unchanged", "unchanged")], false).unwrap();
    assert_eq!(result, "unchanged");
}

#[test]
fn multiline_old_text() {
    let content = "line1\nline2\nline3";
    let result = match_and_apply(content, &[edit("line2\nline3", "new2\nnew3")], false).unwrap();
    assert_eq!(result, "line1\nnew2\nnew3");
}

#[test]
fn empty_old_text_is_noop() {
    // Empty old_text is a no-op (parameter validation catches this at the
    // EditTool level, but match_and_apply skips it gracefully).
    let result = match_and_apply("hello", &[edit("", "x")], false).unwrap();
    assert_eq!(result, "hello");
}

#[test]
fn single_character_file() {
    let result = match_and_apply("a", &[edit("a", "b")], false).unwrap();
    assert_eq!(result, "b");
}

// ---------------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------------

#[test]
fn not_found_error() {
    let err = match_and_apply("hello", &[edit("xyz", "abc")], false).unwrap_err();
    assert_eq!(err, EditError::NotFound);
}

#[test]
fn ambiguous_error() {
    let err = match_and_apply("ab ab", &[edit("ab", "cd")], false).unwrap_err();
    assert_eq!(err, EditError::Ambiguous(2));
}

#[test]
fn overlapping_edits_error() {
    let err =
        match_and_apply("abcdef", &[edit("abcd", "X"), edit("cdef", "Y")], false).unwrap_err();
    assert_eq!(err, EditError::Overlapping);
}

// ---------------------------------------------------------------------------
// Fuzzy matching — quote normalization
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_curly_quotes_to_straight() {
    let content = "He said \u{201c}hello\u{201d} to me";
    let result = match_and_apply(
        content,
        &[edit("He said \"hello\" to me", "She said \"hi\" to me")],
        false,
    )
    .unwrap();
    assert_eq!(result, "She said \"hi\" to me");
}

// ---------------------------------------------------------------------------
// Fuzzy matching — trailing whitespace
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_trailing_whitespace() {
    let content = "hello world   \nfoo bar   \n";
    let result =
        match_and_apply(content, &[edit("hello world\nfoo bar\n", "X\nY\n")], false).unwrap();
    assert_eq!(result, "X\nY\n");
}

// ---------------------------------------------------------------------------
// Fuzzy matching — NFC normalization
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_nfc_normalization() {
    // é can be encoded as a single code point (U+00E9) or as e + combining
    // accent (U+0065 U+0301). Both NFC-normalize to the same thing.
    let content = "caf\u{0065}\u{0301}"; // e + combining accent
    let old = "caf\u{00e9}"; // single code point
    let result = match_and_apply(content, &[edit(old, "coffee")], false).unwrap();
    assert_eq!(result, "coffee");
}

// ---------------------------------------------------------------------------
// Fuzzy matching — combined quotes + whitespace
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_combined_quotes_and_whitespace() {
    let content = "She said \u{201c}hi there\u{201d}  \nbye   \n";
    let result = match_and_apply(
        content,
        &[edit("She said \"hi there\"\nbye\n", "OK\nDone\n")],
        false,
    )
    .unwrap();
    assert_eq!(result, "OK\nDone\n");
}

// ---------------------------------------------------------------------------
// Fuzzy matching — exact match not affected by fuzzy
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_does_not_override_exact() {
    // Exact match should still work when present.
    let content = "hello world";
    let result = match_and_apply(content, &[edit("hello world", "hello rust")], false).unwrap();
    assert_eq!(result, "hello rust");
    // The match should not be fuzzy.
}

// ---------------------------------------------------------------------------
// Fuzzy matching — old text not found
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_not_found_error() {
    let err =
        match_and_apply("no match here", &[edit("completely different", "x")], false).unwrap_err();
    assert_eq!(err, EditError::NotFound);
}

// ---------------------------------------------------------------------------
// Multiple fuzzy edits
// ---------------------------------------------------------------------------

#[test]
fn multiple_fuzzy_edits() {
    let content = "line \u{201c}one\u{201d}\nline \u{201c}two\u{201d}";
    let result = match_and_apply(
        content,
        &[
            edit("line \"one\"", "first"),
            edit("line \"two\"", "second"),
        ],
        false,
    )
    .unwrap();
    assert_eq!(result, "first\nsecond");
}

// ---------------------------------------------------------------------------
// Fuzzy matching — byte range correctness
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_byte_range_correctness_with_unicode() {
    // Content with multi-byte UTF-8 characters before the fuzzy match target.
    // "日本語" takes 9 bytes. Then the target has curly quotes.
    let content = "日本語 says \u{201c}hello\u{201d} end";
    let result = match_and_apply(
        content,
        &[edit("日本語 says \"hello\" end", "replaced")],
        false,
    )
    .unwrap();
    assert_eq!(result, "replaced");
}

// ---------------------------------------------------------------------------
// Fuzzy matching — NFD normalization
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_nfd_normalization() {
    // Content is already NFD (e + combining accent).
    let content = "caf\u{0065}\u{0301}"; // NFD form
    let old = "cafe\u{0301}"; // also NFD but different encoding
                              // Both should normalize to the same NFC form.
    let result = match_and_apply(content, &[edit(old, "coffee")], false).unwrap();
    assert_eq!(result, "coffee");
}

// ---------------------------------------------------------------------------
// Boundary: empty edits
// ---------------------------------------------------------------------------

#[test]
fn empty_edits_returns_original() {
    let result = match_and_apply("unchanged", &[], false).unwrap();
    assert_eq!(result, "unchanged");
}
