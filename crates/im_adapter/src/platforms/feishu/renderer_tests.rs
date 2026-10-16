//! Unit tests for the Feishu renderer.
use closeclaw_common::processor::{ContentBlock, DslInstruction, DslParseResult};

use super::renderer::*;
use super::{assert_button, assert_select_menu, assert_single_action, make_selector_inst};

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

#[test]
fn should_use_card_strikethrough_underline_divider() {
    // strikethrough triggers card
    assert!(should_use_card("~~struck~~", false));
    // underline triggers card
    assert!(should_use_card("<u>underlined</u>", false));
    // single-line --- divider triggers card
    assert!(should_use_card("---", false));
    // single-line *** divider triggers card
    assert!(should_use_card("***", false));
    // multiline with --- surrounded by text triggers card
    assert!(should_use_card("before\n---\nafter", false));
    // plain text without markers returns false
    assert!(!should_use_card("struck through", false));
    // expression with == and < but no ~~ or <u> substring returns false
    assert!(!should_use_card("2 == 2 && 3 < 4", false));
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
    let has_text = elements.iter().any(
        |e| matches!(e, CardElement::Markdown { content } if content.contains("[file: doc.pdf]")),
    );
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

#[test]
fn render_selectors_allow_true_produces_select_menu() {
    let inst = make_selector_inst("Pick", "A,B,C", "pick_action");
    let els = render_selectors(&[inst], true);
    let actions = assert_single_action(&els);
    let menu = assert_select_menu(actions);
    assert_eq!(menu.placeholder.content, "Pick");
    assert_eq!(menu.options.len(), 3);
    assert_eq!(menu.options[0].text.content, "A");
    assert_eq!(menu.options[1].text.content, "B");
    assert_eq!(menu.options[2].text.content, "C");
    assert_eq!(menu.action_name.as_deref(), Some("pick_action"));
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
    let menu = assert_select_menu(actions);
    assert!(menu.options.is_empty());
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
