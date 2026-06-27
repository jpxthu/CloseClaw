//! SKILL.md frontmatter parser
//!
//! Parses YAML frontmatter from skill markdown files.
//!
//! Expected format:
//! ```yaml
//! ---
//! name: "skill-name"
//! description: "What this skill does"
//! allowed_tools: ["tool_a", "tool_b"]
//! when_to_use: "Use this when..."
//! context: Inline  # or Agent { agent_id: "..." }
//! agent: "coding-agent"
//! agent_id: ""
//! effort: Small
//! paths: []
//! user_invocable: false
//! ---
//! ```

use super::{ParseError, ParsedSkill, SkillManifest};

/// Find the byte range of the skill body (text after closing `---`) in a raw SKILL.md string.
///
/// The function performs the following steps:
/// 1. Strips UTF-8 BOM if present.
/// 2. Trims leading/trailing whitespace.
/// 3. Locates the opening `---` delimiter.
/// 4. Locates the closing `---` delimiter.
/// 5. Returns `Some(start..end)` where `start` is the byte offset right after
///    the closing delimiter (skipping the trailing newline) and `end` is the
///    total byte length of the trimmed string.
///
/// Returns `None` when there is no valid frontmatter block (missing opening or
/// closing `---` delimiter, or no content after the closing delimiter).
pub(crate) fn find_body_range(raw: &str) -> Option<std::ops::Range<usize>> {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let raw = raw.trim();

    // Find opening `---`
    let open_pos = raw.find("---\n").or_else(|| raw.find("---\r\n"))?;

    // Search for closing `---` in the content after the opening delimiter.
    // Skip leading newline(s) after opening `---`.
    let after_open = &raw[open_pos + 3..];
    let content_start = after_open.trim_start_matches('\n').trim_start_matches('\r');
    let trim_len = after_open.len() - content_start.len();
    let content_offset = open_pos + 3 + trim_len;

    // Find closing `---` pattern.
    let close_in_content = content_start
        .find("\n---")
        .or_else(|| content_start.find("\r\n---"))
        .or_else(|| content_start.find("---"))?;

    // close_in_content may point to `\n---` or bare `---`.
    // Locate the actual `---` within that region.
    let close_region = &content_start[close_in_content..];
    let dash_offset = close_region
        .find("---")
        .expect("close region must contain ---");
    let body_start = content_offset + close_in_content + dash_offset + 3;

    Some(body_start..raw.len())
}

/// Extract the skill body (instruction text) from a SKILL.md raw string.
///
/// Returns the text after the closing `---` delimiter of the frontmatter,
/// trimmed of leading/trailing whitespace. Returns an empty string when
/// no frontmatter is present or when the frontmatter block has no body.
pub fn extract_skill_body(raw: &str) -> &str {
    // Pre-process: strip BOM and trim so the Range from find_body_range
    // (which is relative to its own BOM-stripped, trimmed copy) is valid.
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let raw = raw.trim();

    find_body_range(raw)
        .map(|range| raw[range].trim())
        .unwrap_or("")
}

/// Parse a SKILL.md file, extracting YAML frontmatter.
pub fn parse_skill_md(raw: &str) -> Result<ParsedSkill, ParseError> {
    // Pre-process: strip BOM and trim, same as find_body_range expects.
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let raw = raw.trim();

    // Require an opening `---` delimiter.
    let has_open = raw.find("---\n").or_else(|| raw.find("---\r\n")).is_some();
    if !has_open {
        return Err(ParseError::MissingDelimiter);
    }

    // Use find_body_range to locate the body (after closing `---`).
    let body_range = find_body_range(raw);

    // Extract frontmatter: everything between opening `---` and closing `---`.
    // find_body_range guarantees the opening `---` exists when it returns Some,
    // and when it returns None (no closing `---`) the entire content after the
    // opening delimiter is frontmatter.
    let frontmatter = match body_range {
        Some(ref range) => {
            // range.start is right after the closing `---`. Extract the
            // frontmatter between the opening and closing delimiters.
            let fm_with_close = &raw[..range.start];
            // The closing `---` may be indented (e.g. `  ---`).
            // Find the last `---` preceded by a newline + optional whitespace.
            if let Some(dash_pos) = fm_with_close.rfind("---") {
                if let Some(nl_pos) = fm_with_close[..dash_pos].rfind('\n') {
                    &fm_with_close[..nl_pos]
                } else {
                    fm_with_close // bare `---` at start (edge case)
                }
            } else {
                fm_with_close
            }
        }
        None => {
            // No closing `---` — treat entire content as frontmatter, body empty.
            raw
        }
    };

    // Strip opening `---` and any trailing newline/whitespace.
    let frontmatter = &frontmatter[3..];
    let frontmatter_trimmed = frontmatter.trim();

    let manifest: SkillManifest = serde_yaml::from_str(frontmatter_trimmed)
        .map_err(|e| ParseError::InvalidYaml(e.to_string()))?;

    if manifest.description.is_empty() {
        return Err(ParseError::MissingDescription);
    }

    // description_only = true when no fields beyond description are present
    let description_only = !frontmatter_trimmed.contains("allowed_tools:")
        && !frontmatter_trimmed.contains("when_to_use:")
        && !frontmatter_trimmed.contains("context:")
        && !frontmatter_trimmed.contains("agent:")
        && !frontmatter_trimmed.contains("agent_id:")
        && !frontmatter_trimmed.contains("effort:")
        && !frontmatter_trimmed.contains("paths:")
        && !frontmatter_trimmed.contains("user_invocable:");

    // Extract body using find_body_range's Range.
    let body = body_range
        .map(|range| raw[range].trim())
        .unwrap_or("")
        .to_string();

    Ok(ParsedSkill {
        manifest,
        description_only,
        frontmatter_raw: frontmatter_trimmed.to_string(),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_frontmatter() {
        let input = r#"---
name: "test-skill"
description: "A test skill for unit testing"
allowed_tools: ["tool_a", "tool_b"]
when_to_use: "Use this when you need to test things"
context: Inline
agent: "coding-agent"
agent_id: ""
effort: Small
paths: []
user_invocable: true
---

# Test Skill
"#;

        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.manifest.name, "test-skill");
        assert_eq!(result.manifest.description, "A test skill for unit testing");
        assert_eq!(result.manifest.allowed_tools, &["tool_a", "tool_b"]);
        assert!(!result.description_only);
    }

    #[test]
    fn test_parse_description_only() {
        let input = r#"---
description: Just a simple skill
---

# Simple Skill
"#;

        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.manifest.description, "Just a simple skill");
        assert!(result.description_only);
    }

    #[test]
    fn test_parse_minimal_frontmatter() {
        let input = r#"---
description: Minimal skill
---

# Minimal
"#;

        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.manifest.description, "Minimal skill");
        assert!(result.description_only);
    }

    #[test]
    fn test_parse_missing_delimiter() {
        let input = "description: No delimiter here";
        let err = parse_skill_md(input).unwrap_err();
        assert_eq!(err, ParseError::MissingDelimiter);
    }

    #[test]
    fn test_parse_invalid_yaml() {
        let input = r#"---
description: [invalid yaml array
---

# Broken
"#;

        let err = parse_skill_md(input).unwrap_err();
        assert!(matches!(err, ParseError::InvalidYaml(_)));
    }

    #[test]
    fn test_parse_missing_description() {
        let input = r#"---
name: "no-description-skill"
---

# No Description
"#;

        let err = parse_skill_md(input).unwrap_err();
        assert_eq!(err, ParseError::MissingDescription);
    }

    #[test]
    fn test_parse_empty_description() {
        let input = r#"---
description: ""
---

# Empty Desc
"#;

        let err = parse_skill_md(input).unwrap_err();
        assert_eq!(err, ParseError::MissingDescription);
    }

    #[test]
    fn test_parse_with_bom() {
        let input = concat!("\u{feff}", "---\ndescription: With BOM\n---\n# Skill\n");
        let result = parse_skill_md(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_whitespace_only_frontmatter() {
        let input = "---\n  \n  description: Whitespace\n  ---\n# Skill\n";
        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.manifest.description, "Whitespace");
    }

    #[test]
    fn test_round_trip_serialization() {
        let input = r#"---
name: "serde-skill"
description: "Testing serde round-trip"
allowed_tools: []
when_to_use: ""
context: Inline
agent: ""
agent_id: ""
effort: Unknown
paths: []
user_invocable: false
---

# Serde Skill
"#;

        let parsed = parse_skill_md(input).expect("parse ok");
        let yaml = serde_yaml::to_string(&parsed.manifest).expect("serialize ok");
        assert!(yaml.contains("serde-skill"));
    }

    #[test]
    fn test_extract_skill_body_standard() {
        let input = concat!(
            "---\n",
            "name: \"test\"\n",
            "description: \"A test skill\"\n",
            "---\n",
            "\n",
            "# Title\n",
            "\n",
            "Some instructions here.\n",
        );

        let body = extract_skill_body(input);
        assert_eq!(body, "# Title\n\nSome instructions here.");
    }

    #[test]
    fn test_extract_skill_body_no_frontmatter() {
        let input = "Just some text with no frontmatter.";
        let body = extract_skill_body(input);
        assert_eq!(body, "");
    }

    #[test]
    fn test_extract_skill_body_no_body() {
        let input = "---\ndescription: A skill\n---\n";
        let body = extract_skill_body(input);
        assert_eq!(body, "");
    }

    #[test]
    fn test_extract_skill_body_with_bom() {
        let input = concat!("\u{feff}", "---\ndescription: With BOM\n---\n# Body\n");
        let body = extract_skill_body(input);
        assert_eq!(body, "# Body");
    }

    #[test]
    fn test_extract_skill_body_whitespace_trim() {
        let input = "---\ndescription: Skill\n---\n\n  # Title  \n\nContent.  \n  ";
        let body = extract_skill_body(input);
        assert_eq!(body, "# Title  \n\nContent.");
    }

    // --- parse_skill_md body extraction tests ---

    #[test]
    fn test_parse_skill_md_populates_body() {
        let input = r#"---
name: "test"
description: "A test skill"
---

# Body

Some instructions here."#;

        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.body, "# Body\n\nSome instructions here.");
    }

    #[test]
    fn test_parse_skill_md_body_no_frontmatter() {
        // No frontmatter → body is empty (extract_skill_body returns "")
        let input = "---\ndescription: test\n---";
        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.body, "");
    }

    #[test]
    fn test_parse_skill_md_body_with_bom() {
        let input = concat!("\u{feff}", "---\ndescription: With BOM\n---\n\n# Body\n");
        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.body, "# Body");
    }

    #[test]
    fn test_parse_skill_md_body_no_body_text() {
        let input = "---\ndescription: No body\n---\n";
        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.body, "");
    }

    #[test]
    fn test_parse_skill_md_body_preserves_multiline() {
        let input = "---\ndescription: Multi\n---\n\n# Step 1\nDo something.\n\n# Step 2\nDo another thing.";
        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(
            result.body,
            "# Step 1\nDo something.\n\n# Step 2\nDo another thing."
        );
    }

    #[test]
    fn test_parse_skill_md_body_bom_no_body() {
        // BOM present, frontmatter present, but no body text after closing ---
        let input = concat!("\u{feff}", "---\ndescription: BOM skill\n---\n");
        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.body, "");
    }

    #[test]
    fn test_parse_skill_md_body_whitespace_only_after_frontmatter() {
        // Only whitespace after closing --- should trim to empty
        let input = "---\ndescription: Whitespace body\n---\n   \n  \n";
        let result = parse_skill_md(input).expect("should parse");
        assert_eq!(result.body, "");
    }

    // --- find_body_range tests ---

    #[test]
    fn test_find_body_range_standard_frontmatter_and_body() {
        let input = "---\nname: test\ndescription: Test\n---\n# Body\n";
        let range = find_body_range(input).expect("should return Some");
        assert!(!range.is_empty(), "range should not be empty");
        // The range includes the newline after closing ---
        assert!(input[range].trim() == "# Body");
    }

    #[test]
    fn test_find_body_range_no_frontmatter() {
        let input = "Just plain text.";
        assert!(find_body_range(input).is_none());
    }

    #[test]
    fn test_find_body_range_with_bom() {
        let input = concat!("\u{feff}", "---\ndescription: With BOM\n---\n# Body\n");
        // find_body_range strips BOM and trims, so index into the same processed string
        let processed = input.strip_prefix('\u{feff}').unwrap_or(input).trim();
        let range = find_body_range(input).expect("should return Some");
        assert!(!range.is_empty(), "range should not be empty");
        // The range includes the newline after closing ---
        assert!(processed[range].trim() == "# Body");
    }

    #[test]
    fn test_find_body_range_frontmatter_only_no_body() {
        let input = "---\ndescription: Skill\n---\n";
        let range = find_body_range(input).expect("should return Some");
        assert!(
            range.is_empty() || input[range].trim().is_empty(),
            "body should be empty after closing ---"
        );
    }

    #[test]
    fn test_extract_skill_body_bom_no_frontmatter() {
        // BOM present but no frontmatter delimiters at all
        let input = concat!("\u{feff}", "Just plain text with BOM.");
        let body = extract_skill_body(input);
        assert_eq!(body, "");
    }
}
