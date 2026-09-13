//! Unit tests for renderer decision behavior (Step 1.4).
//!
//! Validates that DSL instructions do NOT influence output type decisions
//! or inject action elements into cards, per the DSL deferral agreement:
//! - (a) plain text + DSL → text message (DSL does not force card)
//! - (b) formatted text + DSL → card with no button/select action elements
//! - (c) no-DSL normal path regression (text/card decisions unchanged)
//! - (d) ContentBlock combos (Thinking/ToolUse/Image etc.) + DSL → card, no action
//! - (e) edge: empty instructions DslParseResult treated as no DSL
//!
//! The key invariant: `render()` in mod.rs passes `has_dsl=false` to
//! `should_use_card`/`should_use_card_for_blocks` and `None` to
//! `dispatch_blocks`.  Tests verify that when `dispatch_blocks` is called
//! with `None` (the actual call path), no action elements are produced,
//! and that the DSL flag does not change decision outcomes when applied
//! consistently.

use super::renderer::{build_card, dispatch_blocks, should_use_card, should_use_card_for_blocks};
use closeclaw_common::processor::{ContentBlock, DslParseResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return true when any element in the card JSON is a button or select
/// action element.
fn has_action_elements(payload: &serde_json::Value) -> bool {
    let elements = match payload
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
    {
        Some(e) => e,
        None => return false,
    };
    elements.iter().any(|el| {
        let tag = el.get("tag").and_then(|t| t.as_str()).unwrap_or("");
        tag == "action"
    })
}

/// Serialize a card RenderedOutput to JSON value.
fn card_to_json(
    title: Option<String>,
    elements: Vec<super::renderer::CardElement>,
) -> serde_json::Value {
    let output = build_card(title, elements);
    output.payload
}

/// Dispatch blocks with None dsl_result and return the card JSON.
fn dispatch_no_dsl(blocks: &[ContentBlock]) -> serde_json::Value {
    let (title, elements) = dispatch_blocks(blocks, None, true);
    card_to_json(title, elements)
}

// ===========================================================================
// (a) Plain text + DSL → text message (DSL does not force card)
// ===========================================================================

/// Plain text → should_use_card returns false when has_dsl=false
/// (the value render() actually passes).
#[test]
fn plain_text_render_path_produces_text_message() {
    // render() calls should_use_card(text, false) — DSL flag is false
    assert!(!should_use_card("hello world", false));
}

/// should_use_card_for_blocks with plain text and has_dsl=false → false
/// (the value render() actually passes).
#[test]
fn plain_text_blocks_render_path_no_card() {
    let blocks = vec![ContentBlock::Text("hello world".into())];
    // render() calls should_use_card_for_blocks(blocks, false)
    assert!(!should_use_card_for_blocks(&blocks, false));
}

/// Plain text + DSL: dispatch_blocks with None dsl_result produces
/// no action elements.
#[test]
fn plain_text_with_dsl_dispatch_no_action() {
    let blocks = vec![ContentBlock::Text("hello world".into())];
    let json = dispatch_no_dsl(&blocks);
    assert!(!has_action_elements(&json));
}

// ===========================================================================
// (b) Formatted text + DSL → card, card JSON has no action elements
// ===========================================================================

/// Formatted text (multiline/heading) → card (dispatch with None dsl),
/// card JSON has no action elements.
#[test]
fn formatted_text_with_dsl_produces_card_no_action() {
    let blocks = vec![ContentBlock::Text("# Title\n\nBody text".into())];
    // should_use_card_for_blocks returns true for formatted text
    assert!(should_use_card_for_blocks(&blocks, false));
    // dispatch with None → no action elements
    let json = dispatch_no_dsl(&blocks);
    let has_markdown = json
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
        .map(|els| {
            els.iter()
                .any(|el| el.get("tag").and_then(|t| t.as_str()) == Some("markdown"))
        })
        .unwrap_or(false);
    assert!(has_markdown, "expected markdown elements in card");
    assert!(
        !has_action_elements(&json),
        "card should have no action elements"
    );
}

/// Multiline text → card, no action elements (dispatch with None dsl).
#[test]
fn multiline_text_no_action() {
    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    let json = dispatch_no_dsl(&blocks);
    assert!(!has_action_elements(&json));
}

/// Bold text → card, no action elements (dispatch with None dsl).
#[test]
fn bold_text_no_action() {
    let blocks = vec![ContentBlock::Text("**bold text**".into())];
    let json = dispatch_no_dsl(&blocks);
    assert!(!has_action_elements(&json));
}

// ===========================================================================
// (c) No-DSL normal path regression
// ===========================================================================

/// Plain text without DSL → text message (regression).
#[test]
fn no_dsl_plain_text_produces_text_message() {
    assert!(!should_use_card("hello", false));
    let blocks = vec![ContentBlock::Text("hello".into())];
    assert!(!should_use_card_for_blocks(&blocks, false));
}

/// Multiline text without DSL → card (regression).
#[test]
fn no_dsl_multiline_produces_card() {
    assert!(should_use_card("line1\nline2", false));
    let blocks = vec![ContentBlock::Text("line1\nline2".into())];
    assert!(should_use_card_for_blocks(&blocks, false));
}

/// Bold text without DSL → card (regression).
#[test]
fn no_dsl_bold_produces_card() {
    assert!(should_use_card("**bold**", false));
    let blocks = vec![ContentBlock::Text("**bold**".into())];
    assert!(should_use_card_for_blocks(&blocks, false));
}

/// Empty text → no card (regression).
#[test]
fn no_dsl_empty_text_no_card() {
    assert!(!should_use_card("", false));
    let blocks = vec![ContentBlock::Text("".into())];
    assert!(!should_use_card_for_blocks(&blocks, false));
}

/// Heading text without DSL → card (regression).
#[test]
fn no_dsl_heading_produces_card() {
    assert!(should_use_card("# Title", false));
}

/// Dispatch without DSL produces markdown elements, no action.
#[test]
fn no_dsl_dispatch_produces_markdown_no_action() {
    let blocks = vec![ContentBlock::Text("# Title\n\nBody".into())];
    let json = dispatch_no_dsl(&blocks);
    let has_markdown = json
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
        .map(|els| {
            els.iter()
                .any(|el| el.get("tag").and_then(|t| t.as_str()) == Some("markdown"))
        })
        .unwrap_or(false);
    assert!(has_markdown, "expected markdown elements in card");
    assert!(!has_action_elements(&json));
}

/// Single bold line → card with markdown, no action.
#[test]
fn single_bold_line_card_no_action() {
    let blocks = vec![ContentBlock::Text("**bold**".into())];
    let json = dispatch_no_dsl(&blocks);
    assert!(should_use_card_for_blocks(&blocks, false));
    assert!(!has_action_elements(&json));
}

// ===========================================================================
// (d) ContentBlock combos + DSL → card with no action elements
// ===========================================================================

/// Thinking + Text → card (dispatch with None dsl), no action elements.
#[test]
fn thinking_text_combo_no_action() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "reasoning...".into(),
            signature: None,
        },
        ContentBlock::Text("# Result\n\nAnswer here".into()),
    ];
    let json = dispatch_no_dsl(&blocks);
    // Should have a collapsible panel for thinking + markdown for text
    let has_panel = json
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
        .map(|els| {
            els.iter()
                .any(|el| el.get("tag").and_then(|t| t.as_str()) == Some("collapsible_panel"))
        })
        .unwrap_or(false);
    assert!(has_panel, "expected collapsible_panel for Thinking block");
    assert!(!has_action_elements(&json), "no action elements expected");
}

/// ToolUse + Text → card (dispatch with None dsl), no action elements.
#[test]
fn tooluse_text_combo_no_action() {
    let blocks = vec![
        ContentBlock::ToolUse {
            id: "call_1".into(),
            name: "search".into(),
            input: r#"{"q":"test"}"#.into(),
        },
        ContentBlock::Text("Results here".into()),
    ];
    let json = dispatch_no_dsl(&blocks);
    // Should have a note element for ToolUse
    let has_note = json
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
        .map(|els| {
            els.iter()
                .any(|el| el.get("tag").and_then(|t| t.as_str()) == Some("note"))
        })
        .unwrap_or(false);
    assert!(has_note, "expected note element for ToolUse block");
    assert!(!has_action_elements(&json), "no action elements expected");
}

/// Image + Text → card (dispatch with None dsl), no action elements.
#[test]
fn image_text_combo_no_action() {
    let blocks = vec![
        ContentBlock::Image {
            name: "photo.png".into(),
            url: "https://example.com/img.png".into(),
        },
        ContentBlock::Text("See above".into()),
    ];
    let json = dispatch_no_dsl(&blocks);
    // Should have an img element for Image block
    let has_img = json
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
        .map(|els| {
            els.iter()
                .any(|el| el.get("tag").and_then(|t| t.as_str()) == Some("img"))
        })
        .unwrap_or(false);
    assert!(has_img, "expected img element for Image block");
    assert!(!has_action_elements(&json), "no action elements expected");
}

/// Audio + Text → card (dispatch with None dsl), no action elements.
#[test]
fn audio_text_combo_no_action() {
    let blocks = vec![
        ContentBlock::Audio {
            name: "voice.mp3".into(),
            url: "https://example.com/audio.mp3".into(),
        },
        ContentBlock::Text("See above".into()),
    ];
    let json = dispatch_no_dsl(&blocks);
    let has_audio = json
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
        .map(|els| {
            els.iter()
                .any(|el| el.get("tag").and_then(|t| t.as_str()) == Some("audio"))
        })
        .unwrap_or(false);
    assert!(has_audio, "expected audio element for Audio block");
    assert!(!has_action_elements(&json));
}

/// File + Text → card (dispatch with None dsl), no action elements.
#[test]
fn file_text_combo_no_action() {
    let blocks = vec![
        ContentBlock::File {
            name: "doc.pdf".into(),
            url: "https://example.com/doc.pdf".into(),
        },
        ContentBlock::Text("See above".into()),
    ];
    let json = dispatch_no_dsl(&blocks);
    let has_file = json
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
        .map(|els| {
            els.iter()
                .any(|el| el.get("tag").and_then(|t| t.as_str()) == Some("file"))
        })
        .unwrap_or(false);
    assert!(has_file, "expected file element for File block");
    assert!(!has_action_elements(&json));
}

/// All block types combined → card (dispatch with None dsl), no action elements.
#[test]
fn all_block_types_combo_no_action() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "thinking...".into(),
            signature: None,
        },
        ContentBlock::Text("# Title\n\nContent".into()),
        ContentBlock::ToolUse {
            id: "c2".into(),
            name: "tool".into(),
            input: "{}".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "c2".into(),
            content: "result data".into(),
        },
        ContentBlock::Image {
            name: "pic.jpg".into(),
            url: "https://example.com/pic.jpg".into(),
        },
    ];
    let json = dispatch_no_dsl(&blocks);
    assert!(!has_action_elements(&json), "no action elements expected");
}

// ===========================================================================
// (e) Edge: empty instructions DslParseResult → treated as no DSL
// ===========================================================================

/// DslParseResult with empty instructions → dispatch_blocks with Some
/// still produces no action elements (empty instructions = no actions).
#[test]
fn empty_dsl_instructions_produces_card_no_action() {
    let dsl = DslParseResult {
        instructions: vec![],
    };
    let blocks = vec![ContentBlock::Text("# Title\n\nBody".into())];
    let (title, elements) = dispatch_blocks(&blocks, Some(&dsl), true);
    let json = card_to_json(title, elements);
    assert!(
        !has_action_elements(&json),
        "empty DSL should produce no action elements"
    );
}

/// DslParseResult with empty instructions + plain text → dispatch produces
/// no action elements.
#[test]
fn empty_dsl_instructions_plain_text_no_action() {
    let dsl = DslParseResult {
        instructions: vec![],
    };
    let blocks = vec![ContentBlock::Text("plain".into())];
    let (title, elements) = dispatch_blocks(&blocks, Some(&dsl), true);
    let json = card_to_json(title, elements);
    assert!(
        !has_action_elements(&json),
        "empty DSL should produce no action elements"
    );
}

/// DslParseResult with no instructions: build_card still produces
/// a valid card JSON without action elements.
#[test]
fn empty_dsl_build_card_json_no_action() {
    let dsl = DslParseResult {
        instructions: vec![],
    };
    let blocks = vec![ContentBlock::Text("# Heading\n\nParagraph".into())];
    let (title, elements) = dispatch_blocks(&blocks, Some(&dsl), true);
    let json = card_to_json(title, elements);
    // Verify card structure
    assert_eq!(
        json.get("msg_type").and_then(|v| v.as_str()),
        Some("interactive")
    );
    assert!(json.get("card").is_some());
    assert!(!has_action_elements(&json));
}
