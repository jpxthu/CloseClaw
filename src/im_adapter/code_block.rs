//! Code-block parsing utilities.
//!
//! Provides [`ContentSegment`] and [`parse_content_segments`] for splitting
//! markdown content into segments that preserve fenced code blocks as single
//! units, enabling downstream renderers (e.g. Feishu) to emit them intact.

// ---------------------------------------------------------------------------
// ContentSegment
// ---------------------------------------------------------------------------

/// A segment of parsed markdown content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentSegment {
    /// A regular markdown line.
    Markdown(String),
    /// A horizontal rule (`---`).
    Hr,
    /// A fenced code block with optional language annotation.
    CodeBlock { language: String, code: String },
}

// ---------------------------------------------------------------------------
// parse_content_segments
// ---------------------------------------------------------------------------

/// Parses `content` into [`ContentSegment`]s.
///
/// - Fenced code blocks (`` ``` `` … `` ``` ``) are collected as a single
///   [`ContentSegment::CodeBlock`].
/// - Outside code blocks: empty lines are skipped, `---` becomes [`Hr`](ContentSegment::Hr),
///   everything else becomes [`Markdown`](ContentSegment::Markdown).
/// - An unclosed fence is treated as regular markdown text.
/// - Backtick fences nested inside a code block are preserved as content.
///   Emit accumulated code-block lines as regular markdown (unclosed fence).
fn flush_unclosed_fence(lang: &str, code_lines: &[&str], segments: &mut Vec<ContentSegment>) {
    let opening = if lang.is_empty() {
        "```".to_string()
    } else {
        format!("```{}", lang)
    };
    segments.push(ContentSegment::Markdown(opening));
    for cl in code_lines {
        segments.push(ContentSegment::Markdown((*cl).to_string()));
    }
}

/// Process a line outside a code block.
fn process_outside_line(line: &str, segments: &mut Vec<ContentSegment>) -> Option<String> {
    let trimmed = line.trim_end();
    if let Some(after_ticks) = trimmed.strip_prefix("```") {
        if after_ticks.is_empty() || !after_ticks.contains(' ') {
            return Some(after_ticks.to_string()); // opening fence
        }
        segments.push(ContentSegment::Markdown(line.to_string()));
    } else if !trimmed.is_empty() {
        if trimmed == "---" {
            segments.push(ContentSegment::Hr);
        } else {
            segments.push(ContentSegment::Markdown(line.to_string()));
        }
    }
    None
}

pub fn parse_content_segments(content: &str) -> Vec<ContentSegment> {
    let mut segments: Vec<ContentSegment> = Vec::new();
    let mut in_code = false;
    let mut lang = String::new();
    let mut code_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        if in_code {
            let trimmed = line.trim_end();
            if trimmed.starts_with("```") && trimmed.len() >= 3 && trimmed.chars().all(|c| c == '`')
            {
                segments.push(ContentSegment::CodeBlock {
                    language: lang.clone(),
                    code: code_lines.join("\n"),
                });
                in_code = false;
                lang.clear();
                code_lines.clear();
            } else {
                code_lines.push(line);
            }
        } else if let Some(opening_lang) = process_outside_line(line, &mut segments) {
            in_code = true;
            lang = opening_lang;
            code_lines.clear();
        }
    }

    if in_code {
        flush_unclosed_fence(&lang, &code_lines, &mut segments);
    }

    segments
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_code_blocks() {
        let segs = parse_content_segments("hello\nworld\n---\nfoo");
        assert_eq!(
            segs,
            vec![
                ContentSegment::Markdown("hello".into()),
                ContentSegment::Markdown("world".into()),
                ContentSegment::Hr,
                ContentSegment::Markdown("foo".into()),
            ]
        );
    }

    #[test]
    fn single_code_block_with_language() {
        let input = "before\n```rust\nfn main() {}\n```\nafter";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![
                ContentSegment::Markdown("before".into()),
                ContentSegment::CodeBlock {
                    language: "rust".into(),
                    code: "fn main() {}".into(),
                },
                ContentSegment::Markdown("after".into()),
            ]
        );
    }

    #[test]
    fn single_code_block_without_language() {
        let input = "```\nhello\n```";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![ContentSegment::CodeBlock {
                language: String::new(),
                code: "hello".into(),
            },]
        );
    }

    #[test]
    fn multiple_code_blocks() {
        let input = "```a\ncode1\n```\ntext\n```b\ncode2\n```";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![
                ContentSegment::CodeBlock {
                    language: "a".into(),
                    code: "code1".into(),
                },
                ContentSegment::Markdown("text".into()),
                ContentSegment::CodeBlock {
                    language: "b".into(),
                    code: "code2".into(),
                },
            ]
        );
    }

    #[test]
    fn unclosed_code_block_falls_back_to_markdown() {
        let input = "```rust\nfn main() {}\nno close";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![
                ContentSegment::Markdown("```rust".into()),
                ContentSegment::Markdown("fn main() {}".into()),
                ContentSegment::Markdown("no close".into()),
            ]
        );
    }

    #[test]
    fn code_block_with_blank_lines_inside() {
        let input = "```\nline1\n\nline3\n```";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![ContentSegment::CodeBlock {
                language: String::new(),
                code: "line1\n\nline3".into(),
            },]
        );
    }

    #[test]
    fn nested_backticks_inside_code_block() {
        // ``` inside a code block acts as a closing fence.
        // So the first ``` opens, the second ``` closes (empty code block),
        // "inner" is markdown, the third ``` opens, the fourth ``` closes (empty code block).
        let input = "```\n```\ninner\n```\n```";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![
                ContentSegment::CodeBlock {
                    language: String::new(),
                    code: String::new(),
                },
                ContentSegment::Markdown("inner".into()),
                ContentSegment::CodeBlock {
                    language: String::new(),
                    code: String::new(),
                },
            ]
        );
    }

    #[test]
    fn empty_code_block() {
        let input = "```\n```";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![ContentSegment::CodeBlock {
                language: String::new(),
                code: String::new(),
            },]
        );
    }

    #[test]
    fn only_code_block() {
        let input = "```python\nprint('hi')\n```";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![ContentSegment::CodeBlock {
                language: "python".into(),
                code: "print('hi')".into(),
            },]
        );
    }
}
