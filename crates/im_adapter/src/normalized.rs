use regex::Regex;
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// URL normalization
// ---------------------------------------------------------------------------

/// Adds `https://` prefix to bare URLs that lack an http/https scheme.
///
/// Skips URLs already inside markdown link syntax `[text](url)`.
pub fn normalize_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let len = text.len();
    let mut i = 0;

    while i < len {
        // Skip non-ASCII bytes (multi-byte UTF-8) — just copy them as a string slice
        if !bytes[i].is_ascii() {
            let start = i;
            i += 1;
            while i < len && !bytes[i].is_ascii() {
                i += 1;
            }
            out.push_str(&text[start..i]);
            continue;
        }

        // Skip markdown links [text](url)
        if bytes[i] == b'[' {
            let mut j = i + 1;
            while j < len && bytes[j] != b']' {
                j += 1;
            }
            if j < len && j + 1 < len && bytes[j + 1] == b'(' {
                let mut k = j + 1;
                while k < len && bytes[k] != b')' {
                    k += 1;
                }
                out.push_str(&text[i..=k]);
                i = k + 1;
                continue;
            }
            out.push('[');
            i += 1;
            continue;
        }

        if i + 4 <= len && &text[i..i + 4] == "www." {
            out.push_str("https://www.");
            i += 4;
            while i < len
                && !bytes[i].is_ascii_whitespace()
                && bytes[i] != b'"'
                && bytes[i] != b'\''
                && bytes[i] != b')'
                && bytes[i] != b']'
            {
                out.push(bytes[i] as char);
                i += 1;
            }
            continue;
        }

        let preceded_by_scheme =
            i >= 3 && bytes[i - 3] == b':' && bytes[i - 2] == b'/' && bytes[i - 1] == b'/';
        if !preceded_by_scheme
            && i > 0
            && !bytes[i - 1].is_ascii_alphanumeric()
            && i < len
            && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'.')
        {
            let start = i;
            let mut j = i;
            while j < len
                && !bytes[j].is_ascii_whitespace()
                && bytes[j] != b'"'
                && bytes[j] != b'\''
                && bytes[j] != b'<'
                && bytes[j] != b')'
                && bytes[j] != b']'
            {
                j += 1;
            }
            let token = &text[start..j];

            if token.contains('.')
                && !token.starts_with("http://")
                && !token.starts_with("https://")
                && !token.starts_with("ftp://")
                && !token.starts_with("file://")
            {
                out.push_str("https://");
                out.push_str(token);
                i = j;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// Code block language hint
// ---------------------------------------------------------------------------

/// Adds ` ```text` language hint to code blocks that lack a language tag.
pub fn add_code_block_language_hint(text: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(\`\`\`)([^\w\n]|$)").unwrap());
    RE.replace_all(text, "```text$1").to_string()
}
