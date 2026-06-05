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

/// Extract the skill body (instruction text) from a SKILL.md raw string.
///
/// Returns the text after the closing `---` delimiter of the frontmatter,
/// trimmed of leading/trailing whitespace. Returns an empty string when
/// no frontmatter is present or when the frontmatter block has no body.
pub fn extract_skill_body(raw: &str) -> &str {
    let raw = raw.trim();

    // Strip UTF-8 BOM if present
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);

    // Find opening `---`
    let start = match raw.find("---\n").or_else(|| raw.find("---\r\n")) {
        Some(s) => s,
        None => return "",
    };

    let after_open = &raw[start + 3..];

    // Find closing `---`
    let after_skip = after_open.trim_start_matches('\n').trim_start_matches('\r');

    let close_pos = match after_skip
        .find("\n---")
        .or_else(|| after_skip.find("\r\n---"))
        .or_else(|| after_skip.find("---"))
    {
        Some(p) => p,
        None => return "",
    };

    // Advance past the closing delimiter
    let after_close = &after_skip[close_pos..];
    let after_close = after_close
        .trim_start_matches('\n')
        .trim_start_matches('\r')
        .trim_start_matches("---");

    after_close.trim()
}

/// Parse a SKILL.md file, extracting YAML frontmatter.
pub fn parse_skill_md(raw: &str) -> Result<ParsedSkill, ParseError> {
    let raw = raw.trim();

    // Strip UTF-8 BOM if present
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);

    // Find opening `---`
    let start = raw
        .find("---\n")
        .or_else(|| raw.find("---\r\n"))
        .ok_or(ParseError::MissingDelimiter)?;

    let after_delim = raw[start + 3..]
        .trim_start_matches('\n')
        .trim_start_matches('\r');

    // Find closing `---`
    let end = after_delim
        .find("\n---")
        .or_else(|| after_delim.find("\r\n---"))
        .or_else(|| after_delim.find("---"));

    let (frontmatter, _body) = match end {
        Some(n) => (&after_delim[..n], &after_delim[n..]),
        None => (after_delim.as_ref(), ""),
    };

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

    Ok(ParsedSkill {
        manifest,
        description_only,
        frontmatter_raw: frontmatter_trimmed.to_string(),
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
}
