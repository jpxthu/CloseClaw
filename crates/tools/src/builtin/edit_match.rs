//! Edit-match engine: non-incremental matching, overlapping checks, and
//! fuzzy-match fallback for the `EditTool`.

use std::ops::Range;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single edit operation: replace every non-overlapping occurrence of
/// `old_text` with `new_text` within a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOp {
    /// The exact text to search for in the original content.
    pub old_text: String,
    /// The replacement text.
    pub new_text: String,
}

/// Result of matching a single `EditOp` against the file content.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// Byte range in the original content that was matched.
    pub byte_range: Range<usize>,
    /// Index into the `edits` slice that produced this match.
    pub edit_index: usize,
    /// `true` if this match was found via fuzzy matching rather than
    /// exact matching.
    pub is_fuzzy: bool,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the edit-match engine.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// The `old_text` was not found in the file content (exact or fuzzy).
    #[error("oldText not found in file")]
    NotFound,

    /// The `old_text` matched `usize` times in the file, but `replace_all`
    /// was not set.
    #[error("oldText matched {0} times; use replace_all or narrow the range")]
    Ambiguous(usize),

    /// Two edits have overlapping byte ranges in the original content.
    #[error("edits have overlapping match regions")]
    Overlapping,

    /// Fuzzy matching was attempted but produced no match.
    #[error("fuzzy match failed for oldText")]
    FuzzyNotFound,
}

// ---------------------------------------------------------------------------
// Core engine – skeleton (Step 1.2)
// ---------------------------------------------------------------------------

/// Apply all `edits` to `content` using non-incremental matching and
/// reverse-order replacement.
///
/// * All `old_text` values are matched against the **original** content
///   (no incremental updates).
/// * Matches are applied from back to front so that byte offsets remain
///   stable.
/// * Each `old_text` must match exactly once unless `replace_all` is set.
/// * Matching regions must not overlap across edits.
///
/// # Errors
///
/// Returns [`EditError`] when matching or overlap checks fail.
pub fn match_and_apply(
    _content: &str,
    _edits: &[EditOp],
    _replace_all: bool,
) -> Result<String, EditError> {
    // Skeleton – real implementation comes in Step 1.3.
    Ok(_content.to_string())
}
