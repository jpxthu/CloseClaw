use super::*;
use crate::llm::types::ContentBlock;
use crate::processor_chain::dsl_parser::DslParseResult;

fn btn(label: &str, action: &str, value: &str) -> DslInstruction {
    DslInstruction::Button {
        label: label.into(),
        action: action.into(),
        value: value.into(),
    }
}

fn text_block(s: &str) -> ContentBlock {
    ContentBlock::Text(s.to_string())
}

#[test]
fn test_platform() {
    let r = FeishuRenderer::new();
    assert_eq!(r.platform(), "feishu");
}

#[test]
fn test_empty_content() {
    let r = FeishuRenderer::new();
    let out = r.render(&[], None);
    assert_eq!(out.msg_type, "text");
    assert_eq!(out.payload["content"]["text"], "");
}

#[test]
fn test_plain_text() {
    let r = FeishuRenderer::new();
    let out = r.render(&[text_block("hello world")], None);
    assert_eq!(out.msg_type, "text");
    assert_eq!(out.payload["content"]["text"], "hello world");
}

#[test]
fn test_rich_formatting() {
    let r = FeishuRenderer::new();
    let out = r.render(&[text_block("**bold** and _italic_")], None);
    assert_eq!(out.msg_type, "interactive");
}

#[test]
fn test_multiline() {
    let r = FeishuRenderer::new();
    let out = r.render(&[text_block("Line 1\nLine 2")], None);
    assert_eq!(out.msg_type, "interactive");
}

#[test]
fn test_header_extraction() {
    let r = FeishuRenderer::new();
    let out = r.render(&[text_block("# My Title\nBody content")], None);
    assert_eq!(out.msg_type, "interactive");
    assert_eq!(out.payload["card"]["header"]["title"], "My Title");
    assert_eq!(out.payload["card"]["header"]["template"], "blue");
}

#[test]
fn test_hr_element() {
    let r = FeishuRenderer::new();
    let out = r.render(&[text_block("Before\n---\nAfter")], None);
    let els = out.payload["card"]["elements"].as_array().unwrap();
    assert!(els.iter().any(|e| e["tag"] == "hr"));
}

#[test]
fn test_dsl_button() {
    let r = FeishuRenderer::new();
    let dsl = DslParseResult {
        clean_content: "Hi".into(),
        instructions: vec![btn("Yes", "y", "1")],
    };
    let out = r.render(&[text_block("Hello")], Some(&dsl));
    assert_eq!(out.msg_type, "interactive");
    let els = out.payload["card"]["elements"].as_array().unwrap();
    let action = els.iter().find(|e| e["tag"] == "action").unwrap();
    assert_eq!(action["actions"][0]["type"], "primary");
}

#[test]
fn test_dsl_multi_buttons() {
    let r = FeishuRenderer::new();
    let dsl = DslParseResult {
        clean_content: "Hi".into(),
        instructions: vec![btn("A", "a", "1"), btn("B", "b", "2")],
    };
    let out = r.render(&[text_block("Hello")], Some(&dsl));
    let els = out.payload["card"]["elements"].as_array().unwrap();
    let action = els.iter().find(|e| e["tag"] == "action").unwrap();
    assert_eq!(action["actions"][0]["type"], "primary");
    assert_eq!(action["actions"][1]["type"], "default");
}

#[test]
fn test_no_dsl_none() {
    let r = FeishuRenderer::new();
    let out = r.render(&[text_block("No DSL here")], None);
    assert_eq!(out.msg_type, "text");
}

// ---- Code block integration tests ----

/// Helper: extract markdown content strings from CardElement::Markdown items.
fn markdown_contents(elements: &[CardElement]) -> Vec<String> {
    elements
        .iter()
        .filter_map(|e| match e {
            CardElement::Markdown { content } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn test_code_block_with_language() {
    let els = FeishuRenderer::to_elements("```rust\nfn main() {}\n```");
    let mds = markdown_contents(&els);
    assert!(
        mds.iter().any(|m| m.contains("```rust")),
        "should contain ```rust marker"
    );
    assert!(
        mds.iter().any(|m| m.contains("fn main()")),
        "should contain code"
    );
}

#[test]
fn test_code_block_without_language() {
    let els = FeishuRenderer::to_elements("```\nsome code\n```");
    let mds = markdown_contents(&els);
    assert!(
        mds.iter().any(|m| m.starts_with("```\n")),
        "should start with ```"
    );
    assert!(
        mds.iter().any(|m| m.contains("some code")),
        "should contain code"
    );
}

#[test]
fn test_multiple_code_blocks_interleaved() {
    let input = "Intro text\n```rust\ncode1\n```\nMiddle text\n```python\ncode2\n```";
    let els = FeishuRenderer::to_elements(input);
    let mds = markdown_contents(&els);
    // Should have: "Intro text", code block 1, "Middle text", code block 2
    assert!(mds.iter().any(|m| m.contains("Intro text")));
    assert!(mds
        .iter()
        .any(|m| m.contains("```rust") && m.contains("code1")));
    assert!(mds.iter().any(|m| m.contains("Middle text")));
    assert!(mds
        .iter()
        .any(|m| m.contains("```python") && m.contains("code2")));
}

#[test]
fn test_code_block_only() {
    let els = FeishuRenderer::to_elements("```js\nconsole.log(1);\n```");
    // Should be exactly one Markdown element (the code block)
    assert_eq!(els.len(), 1);
    match &els[0] {
        CardElement::Markdown { content } => {
            assert!(content.starts_with("```js\n"));
            assert!(content.ends_with("```"));
        }
        other => panic!("expected Markdown, got {:?}", other),
    }
}

#[test]
fn test_empty_code_block() {
    let els = FeishuRenderer::to_elements("```\n\n```");
    let mds = markdown_contents(&els);
    assert!(!mds.is_empty(), "should produce at least one element");
    assert!(
        mds.iter().any(|m| m.contains("```")),
        "should preserve ``` markers"
    );
}

#[test]
fn test_no_code_block_regression() {
    let els = FeishuRenderer::to_elements("Just plain text\nAnother line");
    // No element should contain ``` markers
    for el in &els {
        if let CardElement::Markdown { content } = el {
            assert!(
                !content.contains("```"),
                "plain text should not contain ```"
            );
        }
    }
}

// ---- ContentBlock[] rendering tests (Step 1.3) ----

fn thinking_block(s: &str) -> ContentBlock {
    ContentBlock::Thinking(s.to_string())
}

fn tool_use_block(id: &str, name: &str, input: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.to_string(),
        name: name.to_string(),
        input: input.to_string(),
    }
}

fn tool_result_block(tool_call_id: &str, content: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_call_id: tool_call_id.to_string(),
        content: content.to_string(),
    }
}

/// Helper: collect all string content (markdown + note plain_text) from a card payload.
fn collect_payload_strings(payload: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(els) = payload["card"]["elements"].as_array() {
        for el in els {
            match el["tag"].as_str() {
                Some("markdown") => {
                    if let Some(c) = el["content"].as_str() {
                        out.push(c.to_string());
                    }
                }
                Some("note") => {
                    if let Some(inner) = el["elements"].as_array() {
                        for n in inner {
                            if let Some(c) = n["content"].as_str() {
                                out.push(c.to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[test]
fn test_single_text_block_compat() {
    // Single Text block with simple content → text output, backward compat
    // (no card, payload follows the legacy {"content": {"text": ...}} shape).
    let r = FeishuRenderer::new();
    let out = r.render(&[text_block("hello world")], None);
    assert_eq!(out.msg_type, "text");
    assert_eq!(out.payload["content"]["text"], "hello world");
}

#[test]
fn test_thinking_block() {
    // Single Thinking block → interactive card whose elements contain
    // the quote-block format `> 💭 Thinking\n> {content}`.
    let r = FeishuRenderer::new();
    let out = r.render(&[thinking_block("reasoning...")], None);
    assert_eq!(out.msg_type, "interactive");

    let strings = collect_payload_strings(&out.payload);
    let joined = strings.join("\n");
    assert!(
        joined.contains("> 💭"),
        "expected quote-block marker `> 💭` in elements, got: {joined}"
    );
    assert!(
        joined.contains("Thinking"),
        "expected label `Thinking` in elements, got: {joined}"
    );
    assert!(
        joined.contains("reasoning..."),
        "expected thinking content in elements, got: {joined}"
    );

    // The Thinking block must be rendered as a markdown element (so
    // Feishu interprets the `>` blockquote syntax).
    let els = out.payload["card"]["elements"].as_array().unwrap();
    assert!(
        els.iter().any(|e| e["tag"] == "markdown"),
        "expected at least one markdown element"
    );
}

#[test]
fn test_tool_use_block() {
    // Single ToolUse block → interactive card whose elements surface
    // the tool name (in a `note` element).
    let r = FeishuRenderer::new();
    let out = r.render(
        &[tool_use_block("call_1", "search_web", "{\"q\":\"rust\"}")],
        None,
    );
    assert_eq!(out.msg_type, "interactive");

    let strings = collect_payload_strings(&out.payload);
    let joined = strings.join("\n");
    assert!(
        joined.contains("search_web"),
        "expected tool name `search_web` in elements, got: {joined}"
    );

    // Tool use should be rendered as a `note` element (per design).
    let els = out.payload["card"]["elements"].as_array().unwrap();
    let note = els
        .iter()
        .find(|e| e["tag"] == "note")
        .expect("expected a note element for ToolUse");
    let inner = note["elements"].as_array().unwrap();
    assert!(!inner.is_empty(), "note element must have inner content");
    assert_eq!(inner[0]["tag"], "plain_text");
    assert!(
        inner[0]["content"].as_str().unwrap().contains("search_web"),
        "note content should contain tool name"
    );
}

#[test]
fn test_tool_result_block() {
    // Single ToolResult block → interactive card whose markdown elements
    // contain the result content.
    let r = FeishuRenderer::new();
    let out = r.render(&[tool_result_block("call_1", "42 files matched")], None);
    assert_eq!(out.msg_type, "interactive");

    let strings = collect_payload_strings(&out.payload);
    let joined = strings.join("\n");
    assert!(
        joined.contains("42 files matched"),
        "expected result content in elements, got: {joined}"
    );
    // ToolResult is rendered as a markdown code-block, so the
    // `**Result**` label should appear.
    assert!(
        joined.contains("**Result**"),
        "expected `**Result**` label in markdown, got: {joined}"
    );
}

#[test]
fn test_mixed_blocks() {
    // Text + Thinking + ToolUse → interactive card with multiple element
    // types (markdown body, quote, note).
    let r = FeishuRenderer::new();
    let out = r.render(
        &[
            text_block("# Answer\nHere is the answer."),
            thinking_block("Let me think..."),
            tool_use_block("call_1", "lookup", "{\"k\":\"v\"}"),
        ],
        None,
    );
    assert_eq!(out.msg_type, "interactive");

    // Header extracted from the first Text block.
    assert_eq!(out.payload["card"]["header"]["title"], "Answer");

    let els = out.payload["card"]["elements"].as_array().unwrap();
    let tags: Vec<&str> = els.iter().filter_map(|e| e["tag"].as_str()).collect();
    assert!(
        tags.contains(&"markdown"),
        "expected at least one markdown element, got tags: {tags:?}"
    );
    assert!(
        tags.contains(&"note"),
        "expected a note element for ToolUse, got tags: {tags:?}"
    );

    // All three block contents should appear somewhere in the payload.
    let strings = collect_payload_strings(&out.payload);
    let joined = strings.join("\n");
    assert!(joined.contains("Here is the answer."), "text missing");
    assert!(
        joined.contains("Let me think...") || joined.contains("> 💭"),
        "thinking content / quote marker missing"
    );
    assert!(joined.contains("lookup"), "tool name missing");
}

#[test]
fn test_empty_blocks() {
    // Empty Vec → text output with empty content (preserves legacy behavior).
    let r = FeishuRenderer::new();
    let out = r.render(&[], None);
    assert_eq!(out.msg_type, "text");
    assert_eq!(out.payload["content"]["text"], "");
}

#[test]
fn test_tool_result_truncated() {
    // ToolResult content above the truncation threshold (impl uses 2000)
    // → the rendered markdown must include the truncation marker.
    let r = FeishuRenderer::new();
    // 3000 ASCII chars, well above the 2000-char RESULT_LIMIT.
    let long_content: String = std::iter::repeat('a').take(3000).collect();
    let out = r.render(&[tool_result_block("call_1", &long_content)], None);

    assert_eq!(out.msg_type, "interactive");

    let strings = collect_payload_strings(&out.payload);
    let joined = strings.join("\n");
    assert!(
        joined.contains("结果过长") && joined.contains("已截断"),
        "expected truncation marker (`结果过长`, `已截断`) in elements, got: {}",
        &joined[..joined.len().min(200)]
    );
    // Sanity: the full 3000-char body must NOT be in the payload verbatim
    // (otherwise truncation didn't actually happen).
    assert!(
        !joined.contains(&long_content),
        "full content should have been truncated"
    );
}
