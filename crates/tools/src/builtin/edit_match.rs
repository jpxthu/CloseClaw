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
// Fuzzy matching helpers
// ---------------------------------------------------------------------------

/// Unicode-curve-quote pairs: (straight, left-curve, right-curve).
const QUOTE_MAP: &[(char, char, char)] = &[
    ('"', '\u{201c}', '\u{201d}'),
    ('\'', '\u{2018}', '\u{2019}'),
];

/// Replace curved quotes with straight equivalents for comparison.
fn normalize_quotes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let mut replaced = false;
        for &(straight, left, right) in QUOTE_MAP {
            if ch == left || ch == right {
                out.push(straight);
                replaced = true;
                break;
            }
        }
        if !replaced {
            out.push(ch);
        }
    }
    out
}

/// Strip trailing whitespace from each line.
fn strip_trailing_ws(s: &str) -> String {
    s.lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Perform Unicode NFC normalization.
fn normalize_nfc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect()
}

/// Perform Unicode NFD normalization.
fn normalize_nfd(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfd().collect()
}

/// Normalize a string for fuzzy comparison.
fn fuzzy_normalize(s: &str) -> String {
    normalize_quotes(&strip_trailing_ws(s))
}

/// Build a byte-offset mapping from a normalized string back to the
/// original string.
///
/// For each character position `i` in the normalized string, `mapping[i]`
/// is the byte offset in the original string where that character starts.
///
/// # Panics
/// Panics if `normalized.len() > original.len()`, which should never
/// happen because normalization can only keep or reduce length.
fn build_normalized_to_original_map(original: &str, normalized: &str) -> Vec<usize> {
    // Build a map: for each byte position in `normalized`, the corresponding
    // byte position in `original`.
    //
    // Algorithm: walk both strings character by character. When characters
    // match, record the position. When they don't (e.g. curly→straight
    // quote), record the original position for the current character and
    // advance both iterators.
    let orig_chars: Vec<(usize, char)> = original.char_indices().collect();
    let norm_chars: Vec<(usize, char)> = normalized.char_indices().collect();
    let mut map = Vec::with_capacity(normalized.len());
    let mut oi = 0; // index into orig_chars
    let mut ni = 0; // index into norm_chars

    while ni < norm_chars.len() {
        if oi < orig_chars.len() {
            let (orig_byte, orig_ch) = orig_chars[oi];
            let (norm_byte, norm_ch) = norm_chars[ni];
            let _ = (orig_byte, norm_byte); // used for tracking
            if orig_ch == norm_ch {
                // Characters match — map this normalized byte position to
                // the original byte position.
                map.push(orig_byte);
                oi += 1;
                ni += 1;
            } else {
                // Characters differ (e.g. curly quote vs straight quote).
                // Record the original position for this normalized character
                // and advance both iterators.
                map.push(orig_byte);
                oi += 1;
                ni += 1;
            }
        } else {
            // Original exhausted — fill remaining with original length.
            map.push(original.len());
            ni += 1;
        }
    }
    map
}

/// Find the original byte range `[start, end)` that corresponds to the
/// matched region `normalized_match` found within `normalized_content` at
/// position `norm_offset`.
///
/// `original_content` is the pre-normalization version.
fn normalized_match_to_original_range(
    original_content: &str,
    normalized_content: &str,
    normalized_match: &str,
    norm_offset: usize,
) -> Range<usize> {
    let map = build_normalized_to_original_map(original_content, normalized_content);

    let start = map
        .get(norm_offset)
        .copied()
        .unwrap_or(original_content.len());
    let end_offset = norm_offset + normalized_match.len();
    let end = if end_offset >= map.len() {
        original_content.len()
    } else {
        map[end_offset]
    };
    start..end
}

/// Try fuzzy matching strategies in order. Returns the byte range in the
/// **original** content if found, `None` otherwise.
///
/// Strategies (in order):
/// 1. Quote normalization (curved → straight)
/// 2. Trailing whitespace stripping
/// 3. Combined normalization (quotes + whitespace)
/// 4. NFC normalization
/// 5. NFD normalization
fn fuzzy_find(original: &str, old_text: &str) -> Option<Range<usize>> {
    // Strategy 1: quote normalization (1:1 char mapping, safe offset)
    let norm_content = normalize_quotes(original);
    let norm_old = normalize_quotes(old_text);
    if let Some(offset) = norm_content.find(&norm_old) {
        return Some(normalized_match_to_original_range(
            original,
            &norm_content,
            &norm_old,
            offset,
        ));
    }

    // Strategy 2: trailing whitespace stripping
    let ws_content = strip_trailing_ws(original);
    let ws_old = strip_trailing_ws(old_text);
    if let Some(offset) = ws_content.find(&ws_old) {
        return Some(normalized_match_to_original_range(
            original,
            &ws_content,
            &ws_old,
            offset,
        ));
    }

    // Strategy 3: combined quotes + whitespace normalization
    let combined_content = fuzzy_normalize(original);
    let combined_old = fuzzy_normalize(old_text);
    if let Some(offset) = combined_content.find(&combined_old) {
        return Some(normalized_match_to_original_range(
            original,
            &combined_content,
            &combined_old,
            offset,
        ));
    }

    // Strategy 4: NFC normalization
    let nfc_content = normalize_nfc(original);
    let nfc_old = normalize_nfc(old_text);
    if let Some(offset) = nfc_content.find(&nfc_old) {
        return Some(normalized_match_to_original_range(
            original,
            &nfc_content,
            &nfc_old,
            offset,
        ));
    }

    // Strategy 5: NFD normalization
    let nfd_content = normalize_nfd(original);
    let nfd_old = normalize_nfd(old_text);
    if let Some(offset) = nfd_content.find(&nfd_old) {
        return Some(normalized_match_to_original_range(
            original,
            &nfd_content,
            &nfd_old,
            offset,
        ));
    }

    None
}

// ---------------------------------------------------------------------------
// Core engine
// ---------------------------------------------------------------------------

/// Find all occurrences of `needle` in `haystack`, returning byte offsets.
fn find_all(haystack: &str, needle: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        offsets.push(start + pos);
        start += pos + 1;
    }
    offsets
}

/// Check whether two byte ranges overlap (exclusive of end boundary).
fn ranges_overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

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
    content: &str,
    edits: &[EditOp],
    replace_all: bool,
) -> Result<String, EditError> {
    if edits.is_empty() {
        return Ok(content.to_string());
    }

    // Phase 1: match all edits against the original content (non-incremental).
    let mut matches: Vec<MatchResult> = Vec::new();

    for (idx, edit) in edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            continue;
        }

        let offsets = find_all(content, &edit.old_text);

        match offsets.len() {
            0 => {
                // Exact match failed — try fuzzy.
                if let Some(range) = fuzzy_find(content, &edit.old_text) {
                    matches.push(MatchResult {
                        byte_range: range,
                        edit_index: idx,
                        is_fuzzy: true,
                    });
                } else {
                    return Err(EditError::NotFound);
                }
            }
            1 => {
                let offset = offsets[0];
                let end = offset + edit.old_text.len();
                matches.push(MatchResult {
                    byte_range: offset..end,
                    edit_index: idx,
                    is_fuzzy: false,
                });
            }
            _ if replace_all => {
                // Keep all matches.
                for offset in offsets {
                    let end = offset + edit.old_text.len();
                    matches.push(MatchResult {
                        byte_range: offset..end,
                        edit_index: idx,
                        is_fuzzy: false,
                    });
                }
            }
            _ => {
                return Err(EditError::Ambiguous(offsets.len()));
            }
        }
    }

    // Phase 2: overlap check — sort by start position and check neighbours.
    matches.sort_by_key(|m| m.byte_range.start);
    for window in matches.windows(2) {
        if ranges_overlap(&window[0].byte_range, &window[1].byte_range) {
            return Err(EditError::Overlapping);
        }
    }

    // Phase 3: reverse-order replacement.
    matches.sort_by(|a, b| b.byte_range.start.cmp(&a.byte_range.start));

    let mut result = content.to_string();
    for m in &matches {
        let edit = &edits[m.edit_index];
        let start = m.byte_range.start;
        let end = m.byte_range.end;
        result.replace_range(start..end, &edit.new_text);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "edit_match_tests.rs"]
mod tests;
