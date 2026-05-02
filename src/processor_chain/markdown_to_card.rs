//! MarkdownToCard — outbound processor that renders markdown as Feishu cards.

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Map, Value};

use super::context::{MessageContext, ProcessedMessage};
use super::dsl_parser::{DslInstruction, DslParseResult};
use super::error::ProcessError;
use super::processor::{MessageProcessor, ProcessPhase};

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
// MarkdownToCard
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MarkdownToCard;

impl MarkdownToCard {
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
        let mut els = Vec::new();
        for line in content.lines() {
            let l = line.trim_end();
            if l.is_empty() {
                continue;
            }
            if l == "---" {
                els.push(CardElement::Hr);
            } else {
                els.push(CardElement::Markdown {
                    content: l.to_string(),
                });
            }
        }
        els
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

    /// Parses dsl_result from metadata.
    fn parse_dsl(metadata: &Map<String, Value>) -> Option<DslParseResult> {
        let s = metadata.get("dsl_result")?.as_str()?;
        serde_json::from_str(s).ok()
    }

    fn card_to_json(card: CardPayload) -> String {
        serde_json::to_string(&card)
            .unwrap_or_else(|_| r#"{"msg_type":"text","content":{"text":""}}"#.to_string())
    }
}

/// Checks for inline formatting.
fn contains_inline(s: &str) -> bool {
    s.contains("**")
        || s.contains("__")
        || s.contains('*')
        || s.contains('_')
        || s.contains('`')
        || (s.contains('[') && s.contains("]("))
}

/// Returns text payload JSON.
fn to_text(content: &str) -> String {
    serde_json::to_string(&serde_json::json!({
        "msg_type": "text",
        "content": { "text": content }
    }))
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// MessageProcessor impl
// ---------------------------------------------------------------------------

#[async_trait]
impl MessageProcessor for MarkdownToCard {
    fn name(&self) -> &str {
        "markdown_to_card"
    }
    fn priority(&self) -> u8 {
        20
    }
    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Outbound
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        let content = ctx.content.trim();
        if content.is_empty() {
            return Ok(None);
        }

        let dsl = Self::parse_dsl(&ctx.metadata);
        let has_dsl = dsl.as_ref().is_some_and(|r| !r.instructions.is_empty());

        if !Self::should_use_card(content, has_dsl) {
            return Ok(Some(ProcessedMessage {
                content: to_text(content),
                metadata: ctx.metadata.clone(),
                suppress: false,
            }));
        }

        let (title, body) = Self::extract_header(content);
        let elements = Self::to_elements(&body);
        let mut all = elements;

        if let Some(r) = dsl {
            all.extend(Self::render_buttons(&r.instructions));
        }

        let header = title.map(|t| CardHeader {
            title: t,
            template: "blue".into(),
        });
        let card = Card {
            header,
            elements: all,
        };
        let payload = CardPayload {
            msg_type: "interactive".into(),
            card,
        };

        Ok(Some(ProcessedMessage {
            content: Self::card_to_json(payload),
            metadata: ctx.metadata.clone(),
            suppress: false,
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn ctx(content: &str, metadata: Map<String, Value>) -> MessageContext {
        MessageContext {
            content: content.into(),
            raw_message_log: vec![],
            metadata,
            skip: false,
        }
    }

    fn metadata_with_dsl(result: DslParseResult) -> Map<String, Value> {
        let json = serde_json::to_string(&result).unwrap();
        let mut m = Map::new();
        m.insert("dsl_result".into(), json.into());
        m
    }

    fn btn(label: &str, action: &str, value: &str) -> DslInstruction {
        DslInstruction::Button {
            label: label.into(),
            action: action.into(),
            value: value.into(),
        }
    }

    #[test]
    fn test_should_use_card_plain() {
        assert!(!MarkdownToCard::should_use_card("hello", false));
    }

    #[test]
    fn test_should_use_card_markdown() {
        assert!(MarkdownToCard::should_use_card("**bold**", false));
    }

    #[test]
    fn test_extract_header() {
        let (t, b) = MarkdownToCard::extract_header("# Title\nBody");
        assert_eq!(t.as_deref(), Some("Title"));
        assert_eq!(b, "Body");
    }

    #[test]
    fn test_extract_no_header() {
        let (t, b) = MarkdownToCard::extract_header("No header");
        assert!(t.is_none());
        assert_eq!(b, "No header");
    }

    #[test]
    fn test_elements_hr() {
        let els = MarkdownToCard::to_elements("a\n---\nb");
        assert!(els.iter().any(|e| matches!(e, CardElement::Hr)));
    }

    #[test]
    fn test_render_buttons_one() {
        let els = MarkdownToCard::render_buttons(&[btn("OK", "a", "v")]);
        match &els[0] {
            CardElement::Action { actions } => assert_eq!(actions[0].action_type, "primary"),
            _ => panic!(),
        }
    }

    #[test]
    fn test_render_buttons_multi() {
        let els = MarkdownToCard::render_buttons(&[btn("A", "a", "1"), btn("B", "b", "2")]);
        match &els[0] {
            CardElement::Action { actions } => {
                assert_eq!(actions[0].action_type, "primary");
                assert_eq!(actions[1].action_type, "default");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_parse_dsl_missing() {
        let m = Map::new();
        assert!(MarkdownToCard::parse_dsl(&m).is_none());
    }

    #[test]
    fn test_parse_dsl_valid() {
        let result = DslParseResult {
            clean_content: "hi".into(),
            instructions: vec![btn("X", "y", "z")],
        };
        let m = metadata_with_dsl(result);
        let parsed = MarkdownToCard::parse_dsl(&m).unwrap();
        assert_eq!(parsed.clean_content, "hi");
    }

    #[tokio::test]
    async fn test_process_text() {
        let c = ctx("hello".into(), Map::new());
        let r = MarkdownToCard.process(&c).await.unwrap().unwrap();
        let v: Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["msg_type"], "text");
    }

    #[tokio::test]
    async fn test_process_markdown() {
        let c = ctx("**bold**".into(), Map::new());
        let r = MarkdownToCard.process(&c).await.unwrap().unwrap();
        let v: Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["msg_type"], "interactive");
    }

    #[tokio::test]
    async fn test_process_header() {
        let c = ctx("# My Title\nBody".into(), Map::new());
        let r = MarkdownToCard.process(&c).await.unwrap().unwrap();
        let v: Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["card"]["header"]["title"], "My Title");
    }

    #[tokio::test]
    async fn test_process_dsl_buttons() {
        let dsl = DslParseResult {
            clean_content: "Hi".into(),
            instructions: vec![btn("Yes", "y", "1"), btn("No", "n", "0")],
        };
        let c = ctx("Hello".into(), metadata_with_dsl(dsl));
        let r = MarkdownToCard.process(&c).await.unwrap().unwrap();
        let v: Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["msg_type"], "interactive");
    }

    #[tokio::test]
    async fn test_process_hr() {
        let c = ctx("Before\n---\nAfter".into(), Map::new());
        let r = MarkdownToCard.process(&c).await.unwrap().unwrap();
        let v: Value = serde_json::from_str(&r.content).unwrap();
        let els = v["card"]["elements"].as_array().unwrap();
        assert!(els.iter().any(|e| e["tag"] == "hr"));
    }

    #[tokio::test]
    async fn test_process_multiline() {
        let c = ctx("Line 1\nLine 2".into(), Map::new());
        let r = MarkdownToCard.process(&c).await.unwrap().unwrap();
        let v: Value = serde_json::from_str(&r.content).unwrap();
        assert_eq!(v["msg_type"], "interactive");
    }

    #[tokio::test]
    async fn test_process_empty() {
        let c = ctx("".into(), Map::new());
        let r = MarkdownToCard.process(&c).await.unwrap();
        assert!(r.is_none());
    }
}
