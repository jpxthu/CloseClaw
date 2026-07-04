//! JSONC comment stripping utilities for agents configuration.

/// Strip `//` line comments from JSONC content.
///
/// Removes everything from `//` to end of line. Does not handle
/// string-embedded comments (e.g., `"foo // bar"`), which is acceptable
/// for agents.json where values are simple strings.
pub(crate) fn strip_jsonc_comments(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            if let Some(idx) = line.find("//") {
                &line[..idx]
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_line_comment() {
        let input = r#"{
            "agents": ["a", "b"]  // trailing comment
        }"#;
        let result = strip_jsonc_comments(input);
        assert_eq!(
            result,
            r#"{
            "agents": ["a", "b"]  
        }"#
        );
    }

    #[test]
    fn test_strip_full_line_comment() {
        let input = r#"{
            // "agents": ["removed"],
            "agents": ["kept"]
        }"#;
        let result = strip_jsonc_comments(input);
        assert_eq!(
            result,
            r#"{
            
            "agents": ["kept"]
        }"#
        );
    }

    #[test]
    fn test_no_comments() {
        let input = r#"{"agents":["x"]}"#;
        let result = strip_jsonc_comments(input);
        assert_eq!(result, input);
    }
}
