//! Unit tests for Feishu text_run style rendering and selector rendering.
use super::post_expand::{expand_element, expand_post_content};
use super::renderer::{render_selectors, CardAction, CardElement};
use closeclaw_common::processor::DslInstruction;

// ================================================================
// text_run style rendering
// ================================================================

#[test]
fn test_text_run_bold_outputs_double_asterisks() {
    let elem = serde_json::json!({"tag": "text_run", "text": "hello", "style": {"bold": true}});
    assert_eq!(expand_element(&elem), "**hello**");
}

#[test]
fn test_text_run_italic_outputs_underscore() {
    let elem = serde_json::json!({"tag": "text_run", "text": "hello", "style": {"italic": true}});
    assert_eq!(expand_element(&elem), "_hello_");
}

#[test]
fn test_text_run_strikethrough_outputs_tildes() {
    let elem =
        serde_json::json!({"tag": "text_run", "text": "hello", "style": {"strikethrough": true}});
    assert_eq!(expand_element(&elem), "~~hello~~");
}

#[test]
fn test_text_run_underline_outputs_u_tag() {
    let elem =
        serde_json::json!({"tag": "text_run", "text": "hello", "style": {"underline": true}});
    assert_eq!(expand_element(&elem), "<u>hello</u>");
}

#[test]
fn test_text_run_bold_plus_strikethrough_combines() {
    let elem = serde_json::json!({
        "tag": "text_run",
        "text": "hello",
        "style": {"bold": true, "strikethrough": true}
    });
    assert_eq!(expand_element(&elem), "~~**hello**~~");
}

#[test]
fn test_text_run_link_outputs_markdown_link() {
    let elem = serde_json::json!({
        "tag": "text_run",
        "text": "click here",
        "style": {"link": {"url": "https://example.com"}}
    });
    assert_eq!(expand_element(&elem), "[click here](https://example.com)");
}

#[test]
fn test_text_run_link_plus_bold_wraps_styled_text() {
    let elem = serde_json::json!({
        "tag": "text_run",
        "text": "click",
        "style": {"bold": true, "link": {"url": "https://example.com"}}
    });
    assert_eq!(expand_element(&elem), "[**click**](https://example.com)");
}

#[test]
fn test_text_run_no_style_outputs_plain_text() {
    let elem = serde_json::json!({"tag": "text_run", "text": "hello"});
    assert_eq!(expand_element(&elem), "hello");
}

#[test]
fn test_text_run_empty_style_outputs_plain_text() {
    let elem = serde_json::json!({"tag": "text_run", "text": "hello", "style": {}});
    assert_eq!(expand_element(&elem), "hello");
}

#[test]
fn test_expand_post_content_preserves_text_run_styles() {
    let post = serde_json::json!({
        "title": "Styled Post",
        "content": [[
            {"tag": "text_run", "text": "bold ", "style": {"bold": true}},
            {"tag": "text_run", "text": "and ", "style": {}},
            {"tag": "text_run", "text": "italic", "style": {"italic": true}}
        ]]
    });
    let result = expand_post_content(&post);
    assert_eq!(result, "Styled Post\n**bold **and _italic_");
}

// ================================================================
// Selector rendering
// ================================================================

fn make_selector_inst(label: &str, options: &str, action: &str) -> DslInstruction {
    DslInstruction {
        instruction_type: "selector".into(),
        params: [
            ("label".into(), label.into()),
            ("options".into(), options.into()),
            ("action".into(), action.into()),
        ]
        .into_iter()
        .collect(),
    }
}

#[test]
fn test_render_selectors_single_selector_returns_select_action() {
    let inst = make_selector_inst("Choose", "A,B,C", "pick");
    let result = render_selectors(&[inst]);
    assert_eq!(result.len(), 1);
    match &result[0] {
        CardElement::Action { actions } => {
            assert_eq!(actions.len(), 1);
            match &actions[0] {
                CardAction::SelectMenu(sel) => {
                    assert_eq!(sel.placeholder.content, "Choose");
                    assert_eq!(sel.options.len(), 3);
                    assert_eq!(sel.options[0].text.content, "A");
                    assert_eq!(sel.options[0].value, "A");
                    assert_eq!(sel.options[2].text.content, "C");
                    assert_eq!(sel.action_name.as_deref(), Some("pick"));
                }
                other => panic!("expected SelectMenu, got {other:?}"),
            }
        }
        other => panic!("expected Action, got {other:?}"),
    }
}

#[test]
fn test_render_selectors_mixed_with_buttons() {
    let btn = DslInstruction {
        instruction_type: "button".into(),
        params: [("label".into(), "OK".into())].into_iter().collect(),
    };
    let sel = make_selector_inst("Pick", "X,Y", "choose");
    let result = render_selectors(&[btn, sel]);
    // render_selectors only processes selector instructions
    assert_eq!(result.len(), 1);
    match &result[0] {
        CardElement::Action { actions } => {
            match &actions[0] {
                CardAction::SelectMenu(sel) => assert_eq!(sel.options.len(), 2),
                other => panic!("expected SelectMenu, got {other:?}"),
            }
        }
        other => panic!("expected Action, got {other:?}"),
    }
}

#[test]
fn test_render_selectors_empty_options_returns_empty_vec() {
    let inst = make_selector_inst("Pick", "", "choose");
    let result = render_selectors(&[inst]);
    assert_eq!(result.len(), 1);
    match &result[0] {
        CardElement::Action { actions } => {
            match &actions[0] {
                CardAction::SelectMenu(sel) => assert_eq!(sel.options.len(), 0),
                other => panic!("expected SelectMenu, got {other:?}"),
            }
        }
        other => panic!("expected Action, got {other:?}"),
    }
}

#[test]
fn test_render_selectors_options_with_spaces_are_trimmed() {
    let inst = make_selector_inst("Pick", " A , B , C ", "choose");
    let result = render_selectors(&[inst]);
    match &result[0] {
        CardElement::Action { actions } => {
            match &actions[0] {
                CardAction::SelectMenu(sel) => {
                    assert_eq!(sel.options.len(), 3);
                    assert_eq!(sel.options[0].text.content, "A");
                    assert_eq!(sel.options[1].text.content, "B");
                    assert_eq!(sel.options[2].text.content, "C");
                }
                other => panic!("expected SelectMenu, got {other:?}"),
            }
        }
        other => panic!("expected Action, got {other:?}"),
    }
}

#[test]
fn test_render_selectors_no_instructions_returns_empty() {
    let result = render_selectors(&[]);
    assert!(result.is_empty());
}

#[test]
fn test_render_selectors_serializes_with_select_static_tag() {
    let inst = make_selector_inst("Choose", "Opt1,Opt2", "my_action");
    let result = render_selectors(&[inst]);
    let json = serde_json::to_value(&result[0]).unwrap();
    assert_eq!(json["tag"], "action");
    let actions = json["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["tag"], "select_static");
    assert_eq!(actions[0]["placeholder"]["content"], "Choose");
    let opts = actions[0]["options"].as_array().unwrap();
    assert_eq!(opts.len(), 2);
    assert_eq!(opts[0]["text"]["content"], "Opt1");
    assert_eq!(opts[0]["value"], "Opt1");
}

#[test]
fn test_render_selectors_no_action_param_omits_action_name() {
    let inst = DslInstruction {
        instruction_type: "selector".into(),
        params: [
            ("label".into(), "Pick".into()),
            ("options".into(), "A,B".into()),
        ]
        .into_iter()
        .collect(),
    };
    let result = render_selectors(&[inst]);
    let json = serde_json::to_value(&result[0]).unwrap();
    let sel = &json["actions"][0];
    assert!(sel.get("action_name").is_none());
}
