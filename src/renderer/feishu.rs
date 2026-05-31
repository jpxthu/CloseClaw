//! Feishu renderer — renders LLM output as Feishu card or text payloads.
//!
//! This module implements the [`Renderer`] trait for the Feishu platform,
//! converting markdown content (with optional DSL buttons) into Feishu
//! interactive card payloads or plain text messages.

use serde::Serialize;

use crate::processor_chain::dsl_parser::{DslInstruction, DslParseResult};

use super::code_block::{parse_content_segments, ContentSegment};
use super::{RenderedOutput, Renderer};

// ---------------------------------------------------------------------------
// Card types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CardPayload {
    pub msg_type: String,
    pub card: Card,
}

#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub header: Option<CardHeader>,
    pub elements: Vec<CardElement>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardHeader {
    pub title: String,
    pub template: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tag")]
pub enum CardElement {
    #[serde(rename = "markdown")]
    Markdown { content: String },
    #[serde(rename = "hr")]
    Hr,
    #[serde(rename = "action")]
    Action { actions: Vec<CardAction> },
}

#[derive(Debug, Clone, Serialize)]
pub struct CardAction {
    pub tag: String,
    pub text: CardText,
    #[serde(rename = "type")]
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardText {
    pub tag: String,
    pub content: String,
}

// ---------------------------------------------------------------------------
// FeishuRenderer
// ---------------------------------------------------------------------------

/// Renderer implementation for Feishu.
#[derive(Debug, Clone, Default)]
pub struct FeishuRenderer;

impl FeishuRenderer {
    pub fn new() -> Self {
        Self
    }

    /// Returns true when content needs a card (has DSL, header, newlines, or inline formatting).
    fn should_use_card(content: &str, has_dsl: bool) -> bool {
        let md = content.trim();
        if md.is_empty() {
            return false;
        }
        if has_dsl || md.starts_with('#') || md.contains('\n') {
            return true;
        }
        contains_inline(md)
    }

    /// Extracts `# Title` from first line.
    fn extract_header(content: &str) -> (Option<String>, String) {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("# ") {
            return (None, content.to_string());
        }
        let end = trimmed.find('\n').unwrap_or(trimmed.len());
        let title = trimmed[2..end].trim().to_string();
        let rest = if end < trimmed.len() {
            trimmed[end + 1..].trim_end().to_string()
        } else {
            String::new()
        };
        (Some(title), rest)
    }

    /// Converts markdown to card elements.
    fn to_elements(content: &str) -> Vec<CardElement> {
        parse_content_segments(content)
            .into_iter()
            .map(|seg| match seg {
                ContentSegment::Markdown(text) => CardElement::Markdown { content: text },
                ContentSegment::Hr => CardElement::Hr,
                ContentSegment::CodeBlock { language, code } => CardElement::Markdown {
                    content: if language.is_empty() {
                        format!("```\n{code}\n```")
                    } else {
                        format!("```{language}\n{code}\n```")
                    },
                },
            })
            .collect()
    }
}

impl FeishuRenderer {
    /// Renders DSL instructions as buttons.
    fn render_buttons(instructions: &[DslInstruction]) -> Vec<CardElement> {
        if instructions.is_empty() {
            return Vec::new();
        }
        let has_primary = instructions
            .iter()
            .any(|i| matches!(i, DslInstruction::Button { .. }));
        let mut actions = Vec::new();
        let mut seen = false;

        for inst in instructions {
            let DslInstruction::Button { label, .. } = inst;
            let bt = if has_primary && !seen {
                seen = true;
                "primary"
            } else {
                "default"
            };
            actions.push(CardAction {
                tag: "button".into(),
                text: CardText {
                    tag: "plain_text".into(),
                    content: label.clone(),
                },
                action_type: bt.into(),
                url: None,
            });
        }
        vec![CardElement::Action { actions }]
    }

    /// Builds an interactive card [`RenderedOutput`].
    fn build_card(title: Option<String>, elements: Vec<CardElement>) -> RenderedOutput {
        let header = title.map(|t| CardHeader {
            title: t,
            template: "blue".into(),
        });
        let card = Card { header, elements };
        let payload = CardPayload {
            msg_type: "interactive".into(),
            card,
        };
        RenderedOutput {
            msg_type: "interactive".into(),
            payload: serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null),
        }
    }

    /// Returns a plain text [`RenderedOutput`].
    fn build_text(content: &str) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({
                "msg_type": "text",
                "content": { "text": content }
            }),
        }
    }
}

fn contains_inline(s: &str) -> bool {
    s.contains("**")
        || s.contains("__")
        || s.contains('*')
        || s.contains('_')
        || s.contains('`')
        || (s.contains('[') && s.contains("]("))
}

impl Renderer for FeishuRenderer {
    fn platform(&self) -> &str {
        "feishu"
    }

    fn render(&self, content: &str, dsl_result: Option<&DslParseResult>) -> RenderedOutput {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Self::build_text("");
        }

        let has_dsl = dsl_result
            .as_ref()
            .is_some_and(|r| !r.instructions.is_empty());

        if !Self::should_use_card(trimmed, has_dsl) {
            return Self::build_text(trimmed);
        }

        let (title, body) = Self::extract_header(trimmed);
        let elements = Self::to_elements(&body);
        let mut all = elements;

        if let Some(r) = dsl_result {
            all.extend(Self::render_buttons(&r.instructions));
        }

        Self::build_card(title, all)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor_chain::dsl_parser::DslParseResult;

    fn btn(label: &str, action: &str, value: &str) -> DslInstruction {
        DslInstruction::Button {
            label: label.into(),
            action: action.into(),
            value: value.into(),
        }
    }

    #[test]
    fn test_platform() {
        let r = FeishuRenderer::new();
        assert_eq!(r.platform(), "feishu");
    }

    #[test]
    fn test_empty_content() {
        let r = FeishuRenderer::new();
        let out = r.render("", None);
        assert_eq!(out.msg_type, "text");
        assert_eq!(out.payload["content"]["text"], "");
    }

    #[test]
    fn test_plain_text() {
        let r = FeishuRenderer::new();
        let out = r.render("hello world", None);
        assert_eq!(out.msg_type, "text");
        assert_eq!(out.payload["content"]["text"], "hello world");
    }

    #[test]
    fn test_rich_formatting() {
        let r = FeishuRenderer::new();
        let out = r.render("**bold** and _italic_", None);
        assert_eq!(out.msg_type, "interactive");
    }

    #[test]
    fn test_multiline() {
        let r = FeishuRenderer::new();
        let out = r.render("Line 1\nLine 2", None);
        assert_eq!(out.msg_type, "interactive");
    }

    #[test]
    fn test_header_extraction() {
        let r = FeishuRenderer::new();
        let out = r.render("# My Title\nBody content", None);
        assert_eq!(out.msg_type, "interactive");
        assert_eq!(out.payload["card"]["header"]["title"], "My Title");
        assert_eq!(out.payload["card"]["header"]["template"], "blue");
    }

    #[test]
    fn test_hr_element() {
        let r = FeishuRenderer::new();
        let out = r.render("Before\n---\nAfter", None);
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
        let out = r.render("Hello", Some(&dsl));
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
        let out = r.render("Hello", Some(&dsl));
        let els = out.payload["card"]["elements"].as_array().unwrap();
        let action = els.iter().find(|e| e["tag"] == "action").unwrap();
        assert_eq!(action["actions"][0]["type"], "primary");
        assert_eq!(action["actions"][1]["type"], "default");
    }

    #[test]
    fn test_no_dsl_none() {
        let r = FeishuRenderer::new();
        let out = r.render("No DSL here", None);
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
}
