//! Feishu renderer — card building and content dispatch logic.

use closeclaw_common::content_segment::{parse_content_segments, ContentSegment};
use closeclaw_common::processor::{ContentBlock, DslInstruction, DslParseResult};
use closeclaw_common::RenderedOutput;
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
pub(crate) struct SelectOption {
    pub(crate) text: CardText,
    pub(crate) value: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SelectMenu {
    pub(crate) placeholder: CardText,
    pub(crate) options: Vec<SelectOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) action_name: Option<String>,
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
    #[serde(rename = "collapsible_panel")]
    CollapsiblePanel {
        header: CollapsiblePanelHeader,
        elements: Vec<CardElement>,
    },
    #[serde(rename = "img")]
    Image { img_key: String, alt: CardText },
    #[serde(rename = "audio")]
    Audio { file_token: String },
    #[serde(rename = "file")]
    File { file_token: String },
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
#[serde(tag = "tag")]
pub(crate) enum CardAction {
    #[serde(rename = "button")]
    Button {
        text: CardText,
        #[serde(rename = "type")]
        action_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    #[serde(rename = "select_static")]
    SelectMenu(SelectMenu),
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CardText {
    pub(crate) tag: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CollapsiblePanelHeader {
    pub(crate) title: CardText,
    pub(crate) icon_tag: String,
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

/// Render a Thinking block as a Feishu collapsible panel.
///
/// The panel defaults to collapsed; users click the header to expand.
/// When `content` is empty, a placeholder is shown inside the panel.
pub(super) fn render_thinking_block(content: &str) -> CardElement {
    let header = CollapsiblePanelHeader {
        title: CardText {
            tag: "plain_text".into(),
            content: "💭 Thinking".into(),
        },
        icon_tag: "down_small_with_solid_bg".into(),
    };
    let inner = if content.is_empty() {
        vec![CardElement::Markdown {
            content: "_（无思考内容）_".into(),
        }]
    } else {
        vec![CardElement::Markdown {
            content: content.to_string(),
        }]
    };
    CardElement::CollapsiblePanel {
        header,
        elements: inner,
    }
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

/// Render a media block (Image/Audio/File) to a card element.
/// When `url` is empty, falls back to a text placeholder.
fn render_media_block(name: &str, url: &str, kind: &str) -> Vec<CardElement> {
    if url.is_empty() {
        to_elements(&format!("[{kind}: {name}]"))
    } else {
        vec![match kind {
            "image" => CardElement::Image {
                img_key: url.to_string(),
                alt: CardText {
                    tag: "plain_text".into(),
                    content: name.to_string(),
                },
            },
            "audio" => CardElement::Audio {
                file_token: url.to_string(),
            },
            _ => CardElement::File {
                file_token: url.to_string(),
            },
        }]
    }
}

/// Dispatch content blocks by type, producing a title and card elements.
pub(crate) fn dispatch_blocks(
    content_blocks: &[ContentBlock],
    dsl_result: Option<&DslParseResult>,
    allow_select_static: bool,
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
            ContentBlock::Thinking {
                thinking: content, ..
            } => {
                elements.push(render_thinking_block(content));
            }
            ContentBlock::ToolUse { name, input, .. } => {
                elements.push(render_tool_use_block(name, input));
            }
            ContentBlock::ToolResult { content, .. } => {
                elements.push(render_tool_result_block(content));
            }
            ContentBlock::Image { name, url } => {
                elements.extend(render_media_block(name, url, "image"));
            }
            ContentBlock::Audio { name, url } => {
                elements.extend(render_media_block(name, url, "audio"));
            }
            ContentBlock::File { name, url } => {
                elements.extend(render_media_block(name, url, "file"));
            }
        }
    }

    if let Some(r) = dsl_result {
        elements.extend(render_buttons(&r.instructions));
        elements.extend(render_selectors(&r.instructions, allow_select_static));
    }

    (title, elements)
}

/// Build button actions for a single selector when `allow_select_static` is false.
///
/// Each option becomes an individual button.  Label format: `{label}: {option}`
/// (or just `{option}` when `label` is empty).  The first button is `primary`;
/// the rest are `default`.
fn render_selector_buttons(
    options: &[String],
    label: &str,
    action_name: Option<String>,
) -> CardElement {
    let _ = action_name; // action is preserved on the selector itself, not buttons
    let mut actions = Vec::new();
    for (idx, opt) in options.iter().enumerate() {
        let bt = if idx == 0 { "primary" } else { "default" };
        let btn_label = if label.is_empty() {
            opt.clone()
        } else {
            format!("{label}: {opt}")
        };
        actions.push(CardAction::Button {
            text: CardText {
                tag: "plain_text".into(),
                content: btn_label,
            },
            action_type: bt.into(),
            url: None,
        });
    }
    CardElement::Action { actions }
}

/// Renders DSL selector instructions as Feishu interactive components.
///
/// When `allow_select_static` is `true`, each selector produces a native
/// `SelectMenu` (`select_static`) inside an `Action` element.
///
/// When `allow_select_static` is `false`, each option is rendered as an
/// individual button via [`render_selector_buttons`].
pub(crate) fn render_selectors(
    instructions: &[DslInstruction],
    allow_select_static: bool,
) -> Vec<CardElement> {
    let selectors: Vec<&DslInstruction> = instructions
        .iter()
        .filter(|i| i.instruction_type == "selector")
        .collect();

    if selectors.is_empty() {
        return Vec::new();
    }
    selectors
        .into_iter()
        .flat_map(|inst| {
            let label = inst.params.get("label").cloned().unwrap_or_default();
            let options_str = inst.params.get("options").cloned().unwrap_or_default();
            let action_name = inst.params.get("action").cloned();
            let options: Vec<String> = options_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            if allow_select_static {
                let select_options: Vec<SelectOption> = options
                    .iter()
                    .map(|opt| SelectOption {
                        text: CardText {
                            tag: "plain_text".into(),
                            content: opt.clone(),
                        },
                        value: opt.clone(),
                    })
                    .collect();
                vec![CardElement::Action {
                    actions: vec![CardAction::SelectMenu(SelectMenu {
                        placeholder: CardText {
                            tag: "plain_text".into(),
                            content: label,
                        },
                        options: select_options,
                        action_name,
                    })],
                }]
            } else {
                vec![render_selector_buttons(&options, &label, action_name)]
            }
        })
        .collect()
}

/// Renders DSL instructions as buttons.
fn render_buttons(instructions: &[DslInstruction]) -> Vec<CardElement> {
    if instructions.is_empty() {
        return Vec::new();
    }
    let has_primary = instructions.iter().any(|i| i.instruction_type == "button");
    let mut actions = Vec::new();
    let mut seen = false;

    for inst in instructions {
        if inst.instruction_type != "button" {
            continue;
        }
        let label = inst.params.get("label").cloned().unwrap_or_default();
        let bt = if has_primary && !seen {
            seen = true;
            "primary"
        } else {
            "default"
        };
        actions.push(CardAction::Button {
            text: CardText {
                tag: "plain_text".into(),
                content: label,
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

/// Extract plain text content from a card JSON payload.
///
/// Traverses `card.elements` and collects `markdown` and `plain_text`
/// element content, joined by newlines.  Returns an empty string when
/// the payload has no extractable text.
pub(crate) fn extract_card_plain_text(payload: &serde_json::Value) -> String {
    let elements = payload
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array());

    let Some(elements) = elements else {
        return String::new();
    };

    let mut lines = Vec::new();
    for el in elements {
        let tag = el.get("tag").and_then(|t| t.as_str()).unwrap_or("");
        match tag {
            "markdown" => {
                if let Some(content) = el.get("content").and_then(|c| c.as_str()) {
                    lines.push(content.to_string());
                }
            }
            "plain_text" => {
                if let Some(content) = el.get("content").and_then(|c| c.as_str()) {
                    lines.push(content.to_string());
                }
            }
            "action" => {
                // Recurse into action.actions[].text.content for buttons
                if let Some(actions) = el.get("actions").and_then(|a| a.as_array()) {
                    for action in actions {
                        if let Some(text) = action.get("text") {
                            if let Some(content) = text.get("content").and_then(|c| c.as_str()) {
                                lines.push(content.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    lines.join("\n")
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
        || s.contains("~~")
        || s.contains("<u>")
        || has_list_or_quote(s)
        || has_divider(s)
}

/// Returns true when any line is a markdown horizontal rule (`---` or `***`).
fn has_divider(s: &str) -> bool {
    s.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "---" || trimmed == "***"
    })
}

/// Returns true when any line starts with a list marker (`- `, `* `, `1. `)
/// or a blockquote marker (`> `).
fn has_list_or_quote(s: &str) -> bool {
    s.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("> ")
            || starts_with_ordered_list(trimmed)
    })
}

/// Checks whether a line starts with an ordered list marker like `1. `,
/// `12. `, etc.
fn starts_with_ordered_list(line: &str) -> bool {
    let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    line[digits.len()..].starts_with(". ")
}
