//! Outbound e2e tests — DslParser → MarkdownToCard full链路.
//
//! Covers: text path, interactive card, DSL buttons, title extraction, full chain.

use std::sync::Arc;

use closeclaw::processor_chain::{
    dsl_parser::DslParser, markdown_to_card::MarkdownToCard, registry::ProcessorRegistry,
    ProcessedMessage,
};

/// Helper: build registry with DslParser + MarkdownToCard in outbound chain.
fn build_registry() -> ProcessorRegistry {
    let mut reg = ProcessorRegistry::new();
    reg.register(Arc::new(DslParser));
    reg.register(Arc::new(MarkdownToCard));
    reg
}

/// Helper: parse outbound result JSON and extract msg_type.
fn msg_type(result: &ProcessedMessage) -> serde_json::Value {
    serde_json::from_str(&result.content).expect("output must be valid JSON")
}

// ── test helpers ──────────────────────────────────────────────────────────────

fn pm(content: &str) -> ProcessedMessage {
    ProcessedMessage {
        content: content.to_string(),
        metadata: serde_json::Map::new(),
        suppress: false,
    }
}

// ── E1: plain text → text msg_type ───────────────────────────────────────────

#[tokio::test]
async fn test_outbound_text_path() {
    // Plain text has no DSL / header / newlines / inline → falls through to text.
    let reg = build_registry();
    let result = reg.process_outbound(pm("Hello world")).await.unwrap();
    let v = msg_type(&result);
    assert_eq!(v["msg_type"], "text", "plain text should stay as text type");
}

// ── E2: bold markdown → interactive card ────────────────────────────────────

#[tokio::test]
async fn test_outbound_interactive_with_formatting() {
    // Bold inline triggers should_use_card → interactive card.
    let reg = build_registry();
    let result = reg
        .process_outbound(pm("**粗体** 和普通文本"))
        .await
        .unwrap();
    let v = msg_type(&result);
    assert_eq!(
        v["msg_type"], "interactive",
        "bold should produce interactive card"
    );
    let elements = v["card"]["elements"]
        .as_array()
        .expect("card must have elements");
    assert!(!elements.is_empty(), "card must have at least one element");
}

// ── E3 / E4: DSL button rendering ───────────────────────────────────────────

#[tokio::test]
async fn test_outbound_dsl_button_rendering() {
    // E3: markdown text + DSL button → first button primary, rest default.
    let reg = build_registry();
    let input = "请确认：\n::button[label:发送;action:send;value:1]\n::button[label:取消;action:cancel;value:0]";
    let result = reg.process_outbound(pm(input)).await.unwrap();
    let v = msg_type(&result);
    assert_eq!(
        v["msg_type"], "interactive",
        "DSL buttons should produce interactive card"
    );

    // Find Action element with buttons.
    let actions: Vec<_> = v["card"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["tag"] == "action")
        .collect();
    assert!(
        !actions.is_empty(),
        "card must contain at least one action element"
    );

    let buttons = &actions[0]["actions"];
    let btns = buttons.as_array().expect("action must have buttons array");
    assert!(btns.len() >= 2, "expected at least 2 buttons");

    // First button is primary, rest are default.
    assert_eq!(btns[0]["type"], "primary", "first button should be primary");
    for btn in btns.iter().skip(1) {
        assert_eq!(
            btn["type"], "default",
            "subsequent buttons should be default"
        );
    }
}

// ── E5: title extraction ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_outbound_card_header_extraction() {
    // "# Title\nBody" → card header.title = "Title".
    let reg = build_registry();
    let result = reg
        .process_outbound(pm("# 文档标题\n\n这是正文内容"))
        .await
        .unwrap();
    let v = msg_type(&result);
    assert_eq!(
        v["msg_type"], "interactive",
        "header markdown should produce interactive card"
    );

    let header = v["card"]["header"]
        .as_object()
        .expect("card must have header");
    assert_eq!(
        header["title"], "文档标题",
        "header title should be extracted correctly"
    );
}

// ── E8: full chain ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_outbound_full_chain() {
    // "# 标题\n\n**粗体** + *斜体*\n\n::button[label:发送;action:send]"
    // → header.title + markdown elements + buttons.
    let reg = build_registry();
    let input = "# 标题\n\n**粗体** + *斜体*\n\n::button[label:发送;action:send]";
    let result = reg.process_outbound(pm(input)).await.unwrap();
    let v = msg_type(&result);

    assert_eq!(
        v["msg_type"], "interactive",
        "full chain should produce interactive card"
    );

    // Header extracted.
    let header = v["card"]["header"]
        .as_object()
        .expect("card must have header");
    assert_eq!(header["title"], "标题", "header title should be '标题'");

    // Elements present (markdown + action).
    let elements = v["card"]["elements"]
        .as_array()
        .expect("card must have elements");
    assert!(!elements.is_empty(), "card must have elements");

    // At least one markdown element with bold content.
    let has_markdown = elements
        .iter()
        .any(|e| e["tag"] == "markdown" && e["content"].as_str().unwrap().contains("**粗体**"));
    assert!(has_markdown, "elements should contain bold markdown");

    // Action element with button exists.
    let has_action = elements.iter().any(|e| e["tag"] == "action");
    assert!(
        has_action,
        "elements should contain action element with button"
    );
}
