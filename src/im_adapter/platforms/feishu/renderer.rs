//! Feishu renderer — card building and content dispatch logic.

use crate::im_adapter::code_block::{parse_content_segments, ContentSegment};
use crate::im_adapter::plugin::RenderedOutput;
use crate::llm::types::ContentBlock;
use crate::processor_chain::dsl_parser::{DslInstruction, DslParseResult};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Card types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CardPayload {
    pub(crate) msg_type: String,
    pub(crate) card: Card,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Card {
    pub(crate) header: Option<CardHeader>,
    pub(crate) elements: Vec<CardElement>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CardHeader {
    pub(crate) title: String,
    pub(crate) template: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tag")]
pub(crate) enum CardElement {
    #[serde(rename = "markdown")]
    Markdown { content: String },
    #[serde(rename = "hr")]
    Hr,
    #[serde(rename = "action")]
    Action { actions: Vec<CardAction> },
    #[serde(rename = "note")]
    Note { elements: Vec<CardNoteElement> },
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CardNoteElement {
    tag: String,
    content: String,
}

impl CardNoteElement {
    fn plain_text(content: impl Into<String>) -> Self {
        Self {
            tag: "plain_text".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CardAction {
    tag: String,
    text: CardText,
    #[serde(rename = "type")]
    action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CardText {
    tag: String,
    content: String,
}

// ---------------------------------------------------------------------------
// Public rendering functions
// ---------------------------------------------------------------------------

/// Returns true when content needs a card (has DSL, header, newlines, or
/// inline formatting).
pub fn should_use_card(content: &str, has_dsl: bool) -> bool {
    let md = content.trim();
    if md.is_empty() {
        return false;
    }
    if has_dsl || md.starts_with('#') || md.contains('\n') {
        return true;
    }
    contains_inline(md)
}

/// Returns true when the structured content blocks warrant an interactive card.
pub fn should_use_card_for_blocks(content_blocks: &[ContentBlock], has_dsl: bool) -> bool {
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
        return should_use_card(text, false);
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

/// Dispatch content blocks by type, producing a title and card elements.
pub(crate) fn dispatch_blocks(
    content_blocks: &[ContentBlock],
    dsl_result: Option<&DslParseResult>,
) -> (Option<String>, Vec<CardElement>) {
    let mut title: Option<String> = None;
    let mut elements: Vec<CardElement> = Vec::new();

    for block in content_blocks {
        match block {
            ContentBlock::Text(text) => {
                if title.is_none() {
                    let (t, body) = extract_header(text.trim());
                    title = t;
                    elements.extend(to_elements(&body));
                } else {
                    elements.extend(to_elements(text.trim()));
                }
            }
            ContentBlock::Thinking(content) => {
                elements.push(render_thinking_block(content));
            }
            ContentBlock::ToolUse { name, input, .. } => {
                elements.push(render_tool_use_block(name, input));
            }
            ContentBlock::ToolResult { content, .. } => {
                elements.push(render_tool_result_block(content));
            }
            ContentBlock::Image(name) => {
                elements.extend(to_elements(&format!("[image: {name}]")));
            }
            ContentBlock::Audio(name) => {
                elements.extend(to_elements(&format!("[audio: {name}]")));
            }
            ContentBlock::File(name) => {
                elements.extend(to_elements(&format!("[file: {name}]")));
            }
        }
    }

    if let Some(r) = dsl_result {
        elements.extend(render_buttons(&r.instructions));
    }

    (title, elements)
}

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
pub(crate) fn build_card(title: Option<String>, elements: Vec<CardElement>) -> RenderedOutput {
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
pub fn build_text(content: &str) -> RenderedOutput {
    RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({
            "msg_type": "text",
            "content": { "text": content }
        }),
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
