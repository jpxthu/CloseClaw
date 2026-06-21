//! Feishu renderer — renders LLM output as Feishu card or text payloads.
//!
//! This module implements the [`Renderer`] trait for the Feishu platform,
//! converting markdown content (with optional DSL buttons) into Feishu
//! interactive card payloads or plain text messages.

use serde::Serialize;

use crate::llm::types::ContentBlock;
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
    /// Feishu card `note` element — used to surface tool-call metadata
    /// (name + truncated input) inline with the response.
    #[serde(rename = "note")]
    Note { elements: Vec<CardNoteElement> },
}

/// Inner element of a [`CardElement::Note`]. Currently only `plain_text`
/// is exposed because that's all Feishu's note element supports.
#[derive(Debug, Clone, Serialize)]
pub struct CardNoteElement {
    pub tag: String,
    pub content: String,
}

impl CardNoteElement {
    pub fn plain_text(content: impl Into<String>) -> Self {
        Self {
            tag: "plain_text".into(),
            content: content.into(),
        }
    }
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
// FeishuRenderer — helpers
// ---------------------------------------------------------------------------

/// Renderer implementation for Feishu.
#[derive(Debug, Clone, Default)]
pub struct FeishuRenderer;

impl FeishuRenderer {
    pub fn new() -> Self {
        Self
    }

    /// Returns true when content needs a card (has DSL, header, newlines, or
    /// inline formatting).
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

    /// Returns true when the structured content blocks warrant an interactive
    /// card.
    fn should_use_card_for_blocks(content_blocks: &[ContentBlock], has_dsl: bool) -> bool {
        if content_blocks.is_empty() {
            return false;
        }
        if has_dsl {
            return true;
        }
        let has_non_text = content_blocks
            .iter()
            .any(|b| !matches!(b, ContentBlock::Text(_)));
        if content_blocks.len() > 1 || has_non_text {
            return true;
        }
        if let ContentBlock::Text(text) = &content_blocks[0] {
            return Self::should_use_card(text, false);
        }
        true
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

// ---------------------------------------------------------------------------
// FeishuRenderer — per-block renderers
// ---------------------------------------------------------------------------

impl FeishuRenderer {
    /// Render a Text block using the legacy markdown → card-element pipeline.
    #[allow(dead_code)] // legacy per-block renderer kept for potential future use
    fn render_text_block(
        &self,
        text: &str,
        dsl_result: Option<&DslParseResult>,
    ) -> (Option<String>, Vec<CardElement>) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return (None, Vec::new());
        }
        let (title, body) = Self::extract_header(trimmed);
        let mut elements = Self::to_elements(&body);
        if let Some(r) = dsl_result {
            elements.extend(Self::render_buttons(&r.instructions));
        }
        (title, elements)
    }

    /// Render a Thinking block as a Feishu markdown quote block.
    fn render_thinking_block(content: &str) -> CardElement {
        let quoted = content
            .lines()
            .map(|line| {
                if line.is_empty() {
                    ">".to_string()
                } else {
                    format!("> {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let body = if quoted.is_empty() {
            "> 💭 Thinking".to_string()
        } else {
            format!("> 💭 Thinking\n{quoted}")
        };
        CardElement::Markdown { content: body }
    }

    /// Render a ToolUse block as a Feishu `note` element.
    fn render_tool_use_block(name: &str, input: &str) -> CardElement {
        const INPUT_PREVIEW_LIMIT: usize = 200;
        let preview: String = input.chars().take(INPUT_PREVIEW_LIMIT).collect();
        let truncated = input.chars().count() > INPUT_PREVIEW_LIMIT;
        let summary = if truncated {
            format!("{preview}…")
        } else {
            preview
        };
        let line = if summary.is_empty() {
            format!("🔧 {name}")
        } else {
            format!("🔧 {name}: {summary}")
        };
        CardElement::Note {
            elements: vec![CardNoteElement::plain_text(line)],
        }
    }

    /// Render a ToolResult block as a markdown element.
    fn render_tool_result_block(content: &str) -> CardElement {
        const RESULT_LIMIT: usize = 2000;
        let char_count = content.chars().count();
        if char_count <= RESULT_LIMIT {
            return CardElement::Markdown {
                content: format!("**Result**\n```\n{content}\n```"),
            };
        }
        let preview: String = content.chars().take(RESULT_LIMIT).collect();
        CardElement::Markdown {
            content: format!(
                "**Result**\n```\n{preview}\n```\n\n\
                 _结果过长，已截断（{char_count} 字符，显示前 {RESULT_LIMIT}）_"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// FeishuRenderer — block dispatch
// ---------------------------------------------------------------------------

impl FeishuRenderer {
    /// Dispatch content blocks by type, producing a title and card elements.
    fn dispatch_blocks(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> (Option<String>, Vec<CardElement>) {
        let mut title: Option<String> = None;
        let mut elements: Vec<CardElement> = Vec::new();

        for block in content_blocks {
            match block {
                ContentBlock::Text(text) => {
                    if title.is_none() {
                        let (t, body) = Self::extract_header(text.trim());
                        title = t;
                        elements.extend(Self::to_elements(&body));
                    } else {
                        elements.extend(Self::to_elements(text.trim()));
                    }
                }
                ContentBlock::Thinking(content) => {
                    elements.push(Self::render_thinking_block(content));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    elements.push(Self::render_tool_use_block(name, input));
                }
                ContentBlock::ToolResult { content, .. } => {
                    elements.push(Self::render_tool_result_block(content));
                }
                ContentBlock::Image(name) => {
                    elements.extend(Self::to_elements(&format!("[image: {}]", name)));
                }
                ContentBlock::Audio(name) => {
                    elements.extend(Self::to_elements(&format!("[audio: {}]", name)));
                }
                ContentBlock::File(name) => {
                    elements.extend(Self::to_elements(&format!("[file: {}]", name)));
                }
            }
        }

        if let Some(r) = dsl_result {
            elements.extend(Self::render_buttons(&r.instructions));
        }

        (title, elements)
    }
}

// ---------------------------------------------------------------------------
// FeishuRenderer — card building
// ---------------------------------------------------------------------------

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
            let DslInstruction::Button { label, .. } = inst else {
                continue;
            };
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

// ---------------------------------------------------------------------------
// Renderer trait impl
// ---------------------------------------------------------------------------

impl Renderer for FeishuRenderer {
    fn platform(&self) -> &str {
        "feishu"
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        if content_blocks.is_empty() {
            return Self::build_text("");
        }

        let has_dsl = dsl_result
            .as_ref()
            .is_some_and(|r| !r.instructions.is_empty());

        // Backward-compat fast path: single Text block, no card needed.
        if content_blocks.len() == 1 {
            if let ContentBlock::Text(text) = &content_blocks[0] {
                if !has_dsl && !Self::should_use_card(text, false) {
                    return Self::build_text(text.trim());
                }
            }
        }

        if !Self::should_use_card_for_blocks(content_blocks, has_dsl) {
            return Self::build_text("");
        }

        let (title, elements) = self.dispatch_blocks(content_blocks, dsl_result);
        Self::build_card(title, elements)
    }
}

// ---------------------------------------------------------------------------
// Tests (separate file)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod feishu_tests;
