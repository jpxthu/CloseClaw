//! Feishu renderer — card building and content dispatch logic.

use crate::code_block::{parse_content_segments, ContentSegment};
use crate::plugin::RenderedOutput;
use closeclaw_common::processor::{ContentBlock, DslInstruction, DslParseResult};
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
    #[allow(dead_code)]
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
fn render_thinking_block(content: &str) -> CardElement {
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

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::processor::ContentBlock;

    #[test]
    fn thinking_block_with_content_produces_collapsible_panel() {
        let el = render_thinking_block("Let me reason...");
        match &el {
            CardElement::CollapsiblePanel { header, elements } => {
                assert_eq!(header.title.content, "💭 Thinking");
                assert_eq!(header.icon_tag, "down_small_with_solid_bg");
                assert_eq!(elements.len(), 1);
                match &elements[0] {
                    CardElement::Markdown { content } => {
                        assert_eq!(content, "Let me reason...");
                    }
                    other => panic!("expected Markdown inside panel, got {other:?}"),
                }
            }
            other => panic!("expected CollapsiblePanel, got {other:?}"),
        }
    }

    #[test]
    fn thinking_block_empty_content_produces_collapsible_panel_with_placeholder() {
        let el = render_thinking_block("");
        match &el {
            CardElement::CollapsiblePanel { header, elements } => {
                assert_eq!(header.title.content, "💭 Thinking");
                assert_eq!(elements.len(), 1);
                match &elements[0] {
                    CardElement::Markdown { content } => {
                        assert!(content.contains("无思考内容"));
                    }
                    other => panic!("expected Markdown placeholder, got {other:?}"),
                }
            }
            other => panic!("expected CollapsiblePanel, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_blocks_with_thinking_includes_collapsible_panel() {
        let blocks = vec![
            ContentBlock::Thinking {
                thinking: "reasoning here".into(),
                signature: None,
            },
            ContentBlock::Text("Hello".into()),
        ];
        let (_, elements) = dispatch_blocks(&blocks, None, true);
        let has_panel = elements
            .iter()
            .any(|e| matches!(e, CardElement::CollapsiblePanel { .. }));
        assert!(has_panel, "expected a CollapsiblePanel in elements");
    }

    #[test]
    fn thinking_block_serializes_with_collapsible_panel_tag() {
        let el = render_thinking_block("some thought");
        let json = serde_json::to_value(&el).unwrap();
        assert_eq!(json["tag"], "collapsible_panel");
        assert_eq!(json["header"]["title"]["content"], "💭 Thinking");
        assert_eq!(json["header"]["icon_tag"], "down_small_with_solid_bg");
        assert!(json["elements"].is_array());
        assert_eq!(json["elements"][0]["tag"], "markdown");
    }

    // ================================================================
    // should_use_card — list and quote marker detection
    // ================================================================

    #[test]
    fn should_use_card_unordered_list_dash() {
        assert!(should_use_card("- buy milk", false));
    }

    #[test]
    fn should_use_card_unordered_list_star() {
        assert!(should_use_card("* item", false));
    }

    #[test]
    fn should_use_card_ordered_list() {
        assert!(should_use_card("1. first item", false));
        assert!(should_use_card("12. twelfth item", false));
    }

    #[test]
    fn should_use_card_blockquote() {
        assert!(should_use_card("> hello", false));
    }

    #[test]
    fn should_use_card_plain_text_returns_false() {
        assert!(!should_use_card("hello world", false));
    }

    #[test]
    fn should_use_card_bold_marker_returns_true() {
        assert!(should_use_card("**bold**", false));
    }

    #[test]
    fn should_use_card_italic_marker_returns_true() {
        assert!(should_use_card("_italic_", false));
    }

    #[test]
    fn should_use_card_code_marker_returns_true() {
        assert!(should_use_card("`code`", false));
    }

    #[test]
    fn should_use_card_link_marker_returns_true() {
        assert!(should_use_card("[link](https://example.com)", false));
    }

    #[test]
    fn should_use_card_multiline_returns_true() {
        assert!(should_use_card("line1\nline2", false));
    }

    #[test]
    fn should_use_card_with_dsl_returns_true() {
        assert!(should_use_card("hello", true));
    }

    #[test]
    fn should_use_card_empty_returns_false() {
        assert!(!should_use_card("", false));
    }

    #[test]
    fn has_list_or_quote_leading_whitespace() {
        assert!(should_use_card("  - indented item", false));
        assert!(should_use_card("  > indented quote", false));
    }

    // ================================================================
    // dispatch_blocks — Image/Audio/File rendering (Step 1.2)
    // ================================================================

    #[test]
    fn dispatch_blocks_image_url_nonempty_produces_native_element() {
        let blocks = vec![ContentBlock::Image {
            name: "photo.png".into(),
            url: "https://example.com/img.png".into(),
        }];
        let (_, elements) = dispatch_blocks(&blocks, None, true);
        assert_eq!(elements.len(), 1);
        match &elements[0] {
            CardElement::Image { img_key, alt } => {
                assert_eq!(img_key, "https://example.com/img.png");
                assert_eq!(alt.content, "photo.png");
            }
            other => panic!("expected CardElement::Image, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_blocks_audio_url_nonempty_produces_native_element() {
        let blocks = vec![ContentBlock::Audio {
            name: "voice.mp3".into(),
            url: "https://example.com/audio.mp3".into(),
        }];
        let (_, elements) = dispatch_blocks(&blocks, None, true);
        assert_eq!(elements.len(), 1);
        match &elements[0] {
            CardElement::Audio { file_token } => {
                assert_eq!(file_token, "https://example.com/audio.mp3");
            }
            other => panic!("expected CardElement::Audio, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_blocks_file_url_nonempty_produces_native_element() {
        let blocks = vec![ContentBlock::File {
            name: "doc.pdf".into(),
            url: "https://example.com/doc.pdf".into(),
        }];
        let (_, elements) = dispatch_blocks(&blocks, None, true);
        assert_eq!(elements.len(), 1);
        match &elements[0] {
            CardElement::File { file_token } => {
                assert_eq!(file_token, "https://example.com/doc.pdf");
            }
            other => panic!("expected CardElement::File, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_blocks_image_url_empty_falls_back_to_text() {
        let blocks = vec![ContentBlock::Image {
            name: "photo.png".into(),
            url: String::new(),
        }];
        let (_, elements) = dispatch_blocks(&blocks, None, true);
        // Empty URL → text placeholder rendered as markdown
        let has_text = elements.iter().any(|e| {
            matches!(e, CardElement::Markdown { content } if content.contains("[image: photo.png]"))
        });
        assert!(has_text, "expected text placeholder for empty URL");
    }

    #[test]
    fn dispatch_blocks_audio_url_empty_falls_back_to_text() {
        let blocks = vec![ContentBlock::Audio {
            name: "voice.mp3".into(),
            url: String::new(),
        }];
        let (_, elements) = dispatch_blocks(&blocks, None, true);
        let has_text = elements.iter().any(|e| {
            matches!(e, CardElement::Markdown { content } if content.contains("[audio: voice.mp3]"))
        });
        assert!(has_text, "expected text placeholder for empty URL");
    }

    #[test]
    fn dispatch_blocks_file_url_empty_falls_back_to_text() {
        let blocks = vec![ContentBlock::File {
            name: "doc.pdf".into(),
            url: String::new(),
        }];
        let (_, elements) = dispatch_blocks(&blocks, None, true);
        let has_text = elements.iter().any(|e| {
            matches!(e, CardElement::Markdown { content } if content.contains("[file: doc.pdf]"))
        });
        assert!(has_text, "expected text placeholder for empty URL");
    }

    // ================================================================
    // CardElement JSON serialization (Step 1.2)
    // ================================================================

    #[test]
    fn card_element_image_serializes_correctly() {
        let el = CardElement::Image {
            img_key: "img_v2_abc".into(),
            alt: CardText {
                tag: "plain_text".into(),
                content: "photo".into(),
            },
        };
        let json = serde_json::to_value(&el).unwrap();
        assert_eq!(json["tag"], "img");
        assert_eq!(json["img_key"], "img_v2_abc");
        assert_eq!(json["alt"]["tag"], "plain_text");
        assert_eq!(json["alt"]["content"], "photo");
    }

    #[test]
    fn card_element_audio_serializes_correctly() {
        let el = CardElement::Audio {
            file_token: "file_token_123".into(),
        };
        let json = serde_json::to_value(&el).unwrap();
        assert_eq!(json["tag"], "audio");
        assert_eq!(json["file_token"], "file_token_123");
    }

    #[test]
    fn card_element_file_serializes_correctly() {
        let el = CardElement::File {
            file_token: "file_token_456".into(),
        };
        let json = serde_json::to_value(&el).unwrap();
        assert_eq!(json["tag"], "file");
        assert_eq!(json["file_token"], "file_token_456");
    }

    // ================================================================
    // render_selectors — allow_select_static (Step 1.3)
    // ================================================================

    fn make_selector_inst(label: &str, options: &str, action: &str) -> DslInstruction {
        let mut params = std::collections::HashMap::new();
        params.insert("label".into(), label.to_string());
        params.insert("options".into(), options.to_string());
        params.insert("action".into(), action.to_string());
        DslInstruction {
            instruction_type: "selector".into(),
            params,
        }
    }

    fn assert_single_action<'a>(els: &'a [CardElement]) -> &'a [CardAction] {
        assert_eq!(els.len(), 1);
        match &els[0] {
            CardElement::Action { actions } => actions,
            other => panic!("expected Action, got {other:?}"),
        }
    }

    fn assert_button(action: &CardAction, label: &str, bt: &str) {
        match action {
            CardAction::Button {
                text,
                action_type,
                url,
            } => {
                assert_eq!(text.content, label);
                assert_eq!(action_type, bt);
                assert!(url.is_none());
            }
            other => panic!("expected Button, got {other:?}"),
        }
    }

    #[test]
    fn render_selectors_allow_true_produces_select_menu() {
        let inst = make_selector_inst("Pick", "A,B,C", "pick_action");
        let els = render_selectors(&[inst], true);
        let actions = assert_single_action(&els);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CardAction::SelectMenu(menu) => {
                assert_eq!(menu.placeholder.content, "Pick");
                assert_eq!(menu.options.len(), 3);
                assert_eq!(menu.options[0].text.content, "A");
                assert_eq!(menu.options[1].text.content, "B");
                assert_eq!(menu.options[2].text.content, "C");
                assert_eq!(menu.action_name.as_deref(), Some("pick_action"));
            }
            other => panic!("expected SelectMenu, got {other:?}"),
        }
    }

    #[test]
    fn render_selectors_allow_false_produces_buttons() {
        let inst = make_selector_inst("Pick", "A,B,C", "pick_action");
        let els = render_selectors(&[inst], false);
        let actions = assert_single_action(&els);
        assert_eq!(actions.len(), 3);
        assert_button(&actions[0], "Pick: A", "primary");
        assert_button(&actions[1], "Pick: B", "default");
        assert_button(&actions[2], "Pick: C", "default");
    }

    #[test]
    fn render_selectors_allow_false_empty_label_uses_option_text() {
        let inst = make_selector_inst("", "X,Y", "do_it");
        let els = render_selectors(&[inst], false);
        let actions = assert_single_action(&els);
        assert_eq!(actions.len(), 2);
        assert_button(&actions[0], "X", "primary");
        assert_button(&actions[1], "Y", "default");
    }

    #[test]
    fn render_selectors_no_selectors_returns_empty() {
        let btn = DslInstruction {
            instruction_type: "button".into(),
            params: std::collections::HashMap::from([("label".into(), "Click".into())]),
        };
        assert!(render_selectors(&[btn], true).is_empty());
    }

    #[test]
    fn render_selectors_empty_options_returns_empty_action() {
        let inst = make_selector_inst("Pick", "", "act");
        let els = render_selectors(&[inst], true);
        let actions = assert_single_action(&els);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CardAction::SelectMenu(menu) => assert!(menu.options.is_empty()),
            other => panic!("expected SelectMenu, got {other:?}"),
        }
    }

    #[test]
    fn render_selectors_allow_false_single_option_primary() {
        let inst = make_selector_inst("Only", "Solo", "only_act");
        let els = render_selectors(&[inst], false);
        let actions = assert_single_action(&els);
        assert_eq!(actions.len(), 1);
        assert_button(&actions[0], "Only: Solo", "primary");
    }

    #[test]
    fn dispatch_blocks_no_selectors_unchanged_with_param() {
        let blocks = vec![ContentBlock::Text("Hello".into())];
        let (_, elements) = dispatch_blocks(&blocks, None, false);
        assert!(elements
            .iter()
            .any(|e| matches!(e, CardElement::Markdown { content } if content == "Hello")));
    }

    #[test]
    fn dispatch_blocks_selector_allow_false_downgrades() {
        let inst = make_selector_inst("Pick", "A,B", "sel_act");
        let dsl = DslParseResult {
            instructions: vec![inst],
        };
        let blocks = vec![ContentBlock::Text("Choose:".into())];
        let (_, elements) = dispatch_blocks(&blocks, Some(&dsl), false);
        let action = elements
            .iter()
            .find(|e| matches!(e, CardElement::Action { .. }));
        assert!(action.is_some(), "expected an Action element");
        match action.unwrap() {
            CardElement::Action { actions } => {
                assert!(
                    actions
                        .iter()
                        .all(|a| matches!(a, CardAction::Button { .. })),
                    "expected only buttons when allow_select_static=false"
                );
            }
            _ => unreachable!(),
        }
    }
}
