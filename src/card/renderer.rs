//! Feishu card renderer — converts card elements to Feishu interactive card JSON.

use super::elements::{
    ButtonElement, ButtonStyle, CardAction, CardElement, ImageElement, MarkdownElement,
    ProgressElement,
};

/// Render a single card element to Feishu JSON format.
pub fn render_element(element: &CardElement) -> serde_json::Value {
    match element {
        CardElement::Markdown(el) => render_markdown(el),
        CardElement::Progress(el) => render_progress(el),
        CardElement::Button(el) => render_button(el),
        CardElement::Divider => serde_json::json!({ "tag": "hr" }),
        CardElement::Image(el) => render_image(el),
    }
}

/// Render a markdown element.
fn render_markdown(el: &MarkdownElement) -> serde_json::Value {
    serde_json::json!({
        "tag": "markdown",
        "content": el.content
    })
}

/// Render a progress bar element.
fn render_progress(el: &ProgressElement) -> serde_json::Value {
    serde_json::json!({
        "tag": "markdown",
        "content": build_progress_text(el.current, el.total, el.labels.as_deref())
    })
}

/// Build the progress bar text representation.
///
/// Format: "▓▓▓░░ 60%\n**步骤**: 3/5 — StepLabel"
pub fn build_progress_text(current: u32, total: u32, labels: Option<&[String]>) -> String {
    let filled = "▓".repeat(current as usize);
    let empty = "░".repeat((total - current) as usize);
    let percentage = if total > 0 {
        (current * 100 / total) as u32
    } else {
        0
    };

    let mut text = format!("**进度**: {}{} {}%\n", filled, empty, percentage);
    text.push_str(&format!("**步骤**: {}/{}", current, total));

    if let Some(labels) = labels {
        if !labels.is_empty() && (current as usize) < labels.len() {
            text.push_str(&format!(" — {}", labels[(current - 1) as usize]));
        }
    }

    text
}

/// Render a button element.
fn render_button(el: &ButtonElement) -> serde_json::Value {
    let button_type = match el.style {
        ButtonStyle::Primary => "primary",
        ButtonStyle::Secondary | ButtonStyle::Default => "default",
    };

    serde_json::json!({
        "tag": "action",
        "actions": [{
            "tag": "button",
            "text": { "tag": "plain_text", "content": el.text },
            "type": button_type
        }]
    })
}

/// Render an image element.
fn render_image(el: &ImageElement) -> serde_json::Value {
    serde_json::json!({
        "tag": "img",
        "img_key": el.url,
        "alt": { "tag": "plain_text", "content": el.alt.as_deref().unwrap_or("") }
    })
}

/// Render a complete card to Feishu interactive card message format.
pub fn render_feishu_card(card: &super::RichCard) -> serde_json::Value {
    use super::{CardHeader, RichCard};

    let elements: Vec<_> = card.elements.iter().map(render_element).collect();

    let mut card_json = serde_json::json!({
        "msg_type": "interactive",
        "card": {
            "elements": elements
        }
    });

    // Add header if present
    if let Some(ref header) = card.header {
        let header_json = serde_json::json!({
            "header": {
                "title": {
                    "tag": "plain_text",
                    "content": header.title.clone()
                },
                "subtitle": header.subtitle.as_ref().map(|s| {
                    serde_json::json!({ "tag": "plain_text", "content": s })
                })
            }
        });
        if let Some(obj) = card_json.as_object_mut() {
            if let Some(card_obj) = obj.get_mut("card").and_then(|c| c.as_object_mut()) {
                card_obj.insert("header".to_string(), header_json["header"].clone());
            }
        }
    }

    card_json
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::elements::{
        ButtonElement, ButtonStyle, CardAction, CardElement, MarkdownElement, ProgressElement,
    };
    use crate::card::{CardHeader, RichCard};

    #[test]
    fn test_render_markdown() {
        let el = MarkdownElement {
            content: "**bold** and _italic_".to_string(),
            collapsible: false,
            collapsed: false,
        };
        let json = render_element(&CardElement::Markdown(el));
        assert_eq!(json["tag"], "markdown");
        assert_eq!(json["content"], "**bold** and _italic_");
    }

    #[test]
    fn test_render_progress() {
        let el = ProgressElement {
            current: 3,
            total: 5,
            labels: Some(vec![
                "分析".to_string(),
                "设计".to_string(),
                "实现".to_string(),
                "测试".to_string(),
                "部署".to_string(),
            ]),
        };
        let json = render_element(&CardElement::Progress(el));
        assert_eq!(json["tag"], "markdown");
        let content = json["content"].as_str().unwrap();
        assert!(content.contains("▓▓▓░░"));
        assert!(content.contains("60%"));
        assert!(content.contains("3/5"));
        assert!(content.contains("实现")); // current step label
    }

    #[test]
    fn test_render_progress_no_labels() {
        let el = ProgressElement {
            current: 2,
            total: 4,
            labels: None,
        };
        let json = render_element(&CardElement::Progress(el));
        let content = json["content"].as_str().unwrap();
        assert!(content.contains("▓▓░░")); // 2 filled + 2 empty = 4 total
        assert!(content.contains("50%"));
        assert!(content.contains("2/4"));
    }

    #[test]
    fn test_render_button_primary() {
        let el = ButtonElement {
            text: "确认".to_string(),
            action: CardAction::Confirm,
            style: ButtonStyle::Primary,
        };
        let json = render_element(&CardElement::Button(el));
        assert_eq!(json["tag"], "action");
        let actions = json["actions"].as_array().unwrap();
        assert_eq!(actions[0]["tag"], "button");
        assert_eq!(actions[0]["type"], "primary");
        assert_eq!(actions[0]["text"]["content"], "确认");
    }

    #[test]
    fn test_render_button_default() {
        let el = ButtonElement {
            text: "取消".to_string(),
            action: CardAction::Cancel,
            style: ButtonStyle::Default,
        };
        let json = render_element(&CardElement::Button(el));
        let actions = json["actions"].as_array().unwrap();
        assert_eq!(actions[0]["type"], "default");
    }

    #[test]
    fn test_render_divider() {
        let json = render_element(&CardElement::Divider);
        assert_eq!(json["tag"], "hr");
    }

    #[test]
    fn test_build_progress_text() {
        let text = build_progress_text(2, 5, None);
        assert!(text.contains("▓▓░░░"));
        assert!(text.contains("40%"));
        assert!(text.contains("2/5"));
    }

    #[test]
    fn test_build_progress_text_with_labels() {
        let labels = vec!["分析".to_string(), "设计".to_string(), "实现".to_string()];
        let text = build_progress_text(2, 3, Some(&labels));
        assert!(text.contains("▓▓░"));
        assert!(text.contains("66%"));
        assert!(text.contains("2/3"));
        assert!(text.contains("设计")); // step 2 label
    }

    #[test]
    fn test_render_feishu_card_with_header() {
        use crate::card::elements::{CardElement, ProgressElement};
        use crate::card::{CardHeader, PlanStep, RichCard, StepStatus};

        let card = RichCard {
            card_id: Some("msg_123".to_string()),
            title: "测试计划".to_string(),
            header: Some(CardHeader {
                title: "测试计划".to_string(),
                subtitle: Some("步骤 1/3".to_string()),
                avatar_url: None,
            }),
            elements: vec![
                CardElement::Progress(ProgressElement {
                    current: 1,
                    total: 3,
                    labels: Some(vec![
                        "第一步".to_string(),
                        "第二步".to_string(),
                        "第三步".to_string(),
                    ]),
                }),
                CardElement::Divider,
            ],
        };

        let json = render_feishu_card(&card);
        assert_eq!(json["msg_type"], "interactive");
        assert_eq!(json["card"]["header"]["title"]["content"], "测试计划");
        assert_eq!(json["card"]["header"]["subtitle"]["content"], "步骤 1/3");
    }

    #[test]
    fn test_render_feishu_card_without_header() {
        use crate::card::elements::{CardElement, MarkdownElement};

        let card = RichCard {
            card_id: None,
            title: "无头卡片".to_string(),
            header: None,
            elements: vec![CardElement::Markdown(MarkdownElement {
                content: "简单内容".to_string(),
                collapsible: false,
                collapsed: false,
            })],
        };

        let json = render_feishu_card(&card);
        assert_eq!(json["msg_type"], "interactive");
        assert!(json["card"].as_object().unwrap().get("header").is_none());
    }
}
