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
/// - Outside code blocks: empty lines are preserved as empty
///   [`Markdown("")`](ContentSegment::Markdown) segments, `---` becomes
///   [`Hr`](ContentSegment::Hr), everything else becomes
///   [`Markdown`](ContentSegment::Markdown).
/// - An unclosed fence is treated as regular markdown text.
/// - A line consisting only of backticks (≥3) inside a code block closes
///   the fence (the backtick line itself is consumed as the closing fence).
///   Emit accumulated code-block lines as regular [`Markdown`](ContentSegment::Markdown)
///   segments (the fence was never closed).
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
    } else if trimmed == "---" {
        segments.push(ContentSegment::Hr);
    } else {
        // Preserve empty lines as empty Markdown segments to maintain
        // original formatting (blank lines between paragraphs, etc.).
        segments.push(ContentSegment::Markdown(line.to_string()));
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

    // ---- Additional edge-case tests (Step 1.3) ----

    #[test]
    fn empty_string_input() {
        let segs = parse_content_segments("");
        assert!(segs.is_empty());
    }

    #[test]
    fn blank_line_outside_code_block_preserved() {
        let input = "line1\n\nline2";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![
                ContentSegment::Markdown("line1".into()),
                ContentSegment::Markdown("".into()),
                ContentSegment::Markdown("line2".into()),
            ]
        );
    }

    #[test]
    fn multiple_hr_sequential() {
        let input = "---\n---\n---";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![ContentSegment::Hr, ContentSegment::Hr, ContentSegment::Hr,]
        );
    }

    #[test]
    fn four_backtick_fence_opens_with_language() {
        // Known design behavior (not a bug): the parser only strips the leading
        // 3 backticks from the opening fence; the 4th backtick is retained as
        // part of the language field, producing language="`rust".
        let input = "````rust\nfn main() {}\n````";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![ContentSegment::CodeBlock {
                language: "`rust".into(),
                code: "fn main() {}".into(),
            },]
        );
    }

    #[test]
    fn mixed_segments_long_content() {
        let input = "# Title\n\nSome introductory text.\n\n```python\ndef hello():\n    print(\"world\")\n\n    return 42\n```\n\n---\n\nMore text after the rule.\n\n```\nraw\ncode\n```\n\nFinal paragraph.";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![
                ContentSegment::Markdown("# Title".into()),
                ContentSegment::Markdown("".into()),
                ContentSegment::Markdown("Some introductory text.".into()),
                ContentSegment::Markdown("".into()),
                ContentSegment::CodeBlock {
                    language: "python".into(),
                    code: "def hello():\n    print(\"world\")\n\n    return 42".into(),
                },
                ContentSegment::Markdown("".into()),
                ContentSegment::Hr,
                ContentSegment::Markdown("".into()),
                ContentSegment::Markdown("More text after the rule.".into()),
                ContentSegment::Markdown("".into()),
                ContentSegment::CodeBlock {
                    language: String::new(),
                    code: "raw\ncode".into(),
                },
                ContentSegment::Markdown("".into()),
                ContentSegment::Markdown("Final paragraph.".into()),
            ]
        );
    }

    #[test]
    fn backtick_fence_with_extra_backticks_inside_code_block() {
        // A line of only backticks (>=3) inside a code block closes it
        // per "全反引号行即关闭围栏" rule. The remaining ``` after line2
        // is an unclosed fence that falls back to Markdown("```").
        let input = "```\nline1\n````\nline2\n```";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![
                ContentSegment::CodeBlock {
                    language: String::new(),
                    code: "line1".into(),
                },
                ContentSegment::Markdown("line2".into()),
                ContentSegment::Markdown("```".into()),
            ]
        );
    }
}
