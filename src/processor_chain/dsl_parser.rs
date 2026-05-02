//! DslParser — outbound [`MessageProcessor`] for parsing `::button[...]` DSL from LLM output.
//!
//! DSL format: `::button[label:X;action:Y;value:Z]`
//! - One instruction per line
//! - Parameters separated by `;`
//! - Each parameter in `key:value` format
//!
//! The parser removes DSL lines from markdown and stores the parsed result
//! in [`MessageContext`] metadata under the `"dsl_result"` key.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase};

// ---------------------------------------------------------------------------
// DSL data types
// ---------------------------------------------------------------------------

/// A parsed DSL instruction extracted from markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DslInstruction {
    /// A clickable button with a label, action identifier, and optional value.
    Button {
        label: String,
        action: String,
        value: String,
    },
}

/// Result of parsing a markdown string for DSL instructions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DslParseResult {
    /// Markdown content with all DSL lines removed (preserving original line order).
    pub clean_content: String,
    /// Extracted DSL instructions in the order they appear in the source.
    pub instructions: Vec<DslInstruction>,
}

// ---------------------------------------------------------------------------
// DslParser
// ---------------------------------------------------------------------------

/// Processor that parses `::button[...]` DSL instructions from outbound LLM output.
///
/// Implements [`MessageProcessor`] with [`ProcessPhase::Outbound`] and priority 10.
#[derive(Debug, Clone, Default)]
pub struct DslParser;

impl DslParser {
    /// Parse DSL instructions from `content` and return a [`DslParseResult`].
    ///
    /// If no DSL lines are found, `instructions` is empty and `clean_content`
    /// equals the original `content`.
    pub fn parse(&self, content: &str) -> DslParseResult {
        let mut instructions: Vec<DslInstruction> = Vec::new();
        let mut clean_lines: Vec<&str> = Vec::new();

        for line in content.lines() {
            if let Some(instruction) = parse_dsl_line(line) {
                instructions.push(instruction);
            } else {
                clean_lines.push(line);
            }
        }

        let clean_content = if instructions.is_empty() {
            // No DSL found — return original content unchanged
            content.to_string()
        } else {
            // Remove DSL lines, preserving line order
            clean_lines.join("\n")
        };

        DslParseResult {
            clean_content,
            instructions,
        }
    }
}

/// Try to parse a single line as a DSL instruction.
///
/// Returns `None` if the line is not a DSL line.
fn parse_dsl_line(line: &str) -> Option<DslInstruction> {
    let trimmed = line.trim();
    if !trimmed.starts_with("::button[") || !trimmed.ends_with(']') {
        return None;
    }

    // Extract content between `[` and `]`
    let start = trimmed.find('[')? + 1;
    let end = trimmed.len() - 1;
    if start >= end {
        return None;
    }
    let inner = &trimmed[start..end];

    // Parse parameters: key:value separated by ';'
    let mut label: Option<String> = None;
    let mut action: Option<String> = None;
    let mut value: Option<String> = None;

    for param in inner.split(';') {
        let param = param.trim();
        if let Some((key, val)) = param.split_once(':') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "label" => label = Some(val.to_string()),
                "action" => action = Some(val.to_string()),
                "value" => value = Some(val.to_string()),
                _ => {}
            }
        }
    }

    let label = label?;
    let action = action?;
    let value = value.unwrap_or_default();

    Some(DslInstruction::Button {
        label,
        action,
        value,
    })
}

#[async_trait]
impl MessageProcessor for DslParser {
    fn name(&self) -> &str {
        "DslParser"
    }

    fn priority(&self) -> u8 {
        10
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Outbound
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<super::ProcessedMessage>, ProcessError> {
        let content = &ctx.content;

        let result = self.parse(content);
        let json = serde_json::to_string(&result)
            .map_err(|e| ProcessError::processor_failed("DslParser", e))?;

        let mut metadata = ctx.metadata.clone();
        metadata.insert("dsl_result".to_string(), serde_json::Value::String(json));

        Ok(Some(super::ProcessedMessage {
            content: result.clean_content,
            metadata,
            suppress: false,
        }))
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_dsl() {
        let parser = DslParser;
        let input = "Hello, this is a normal message without any DSL.";
        let result = parser.parse(input);

        assert!(result.instructions.is_empty());
        assert_eq!(result.clean_content, input);
    }

    #[test]
    fn test_single_dsl() {
        let parser = DslParser;
        let input = "::button[label:Click Me;action:navigate;value:/home]";
        let result = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(
            result.instructions[0],
            DslInstruction::Button {
                label: "Click Me".to_string(),
                action: "navigate".to_string(),
                value: "/home".to_string(),
            }
        );
        assert!(result.clean_content.is_empty());
    }

    #[test]
    fn test_multiple_dsl() {
        let parser = DslParser;
        let input =
            "::button[label:Yes;action:confirm;value:1]\n::button[label:No;action:cancel;value:0]";
        let result = parser.parse(input);

        assert_eq!(result.instructions.len(), 2);
        match &result.instructions[0] {
            DslInstruction::Button { label, .. } => assert_eq!(label, "Yes"),
        }
        match &result.instructions[1] {
            DslInstruction::Button { label, .. } => assert_eq!(label, "No"),
        }
        assert!(result.clean_content.is_empty());
    }

    #[test]
    fn test_dsl_mixed_with_text() {
        let parser = DslParser;
        let input = "Hello world\n::button[label:OK;action:submit;value:yes]\nGoodbye";
        let result = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.clean_content, "Hello world\nGoodbye");
    }

    #[test]
    fn test_dsl_at_first_line() {
        let parser = DslParser;
        let input = "::button[label:Start;action:begin;value:]\nNow the content starts here.";
        let result = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.clean_content, "Now the content starts here.");
    }

    #[test]
    fn test_dsl_at_middle() {
        let parser = DslParser;
        let input = "Before\n::button[label:Middle;action:go;value:x]\nAfter";
        let result = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.clean_content, "Before\nAfter");
    }

    #[test]
    fn test_dsl_at_last_line() {
        let parser = DslParser;
        let input = "Some text here\n::button[label:End;action:finish;value:done]";
        let result = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.clean_content, "Some text here");
    }

    #[test]
    fn test_dsl_param_with_spaces() {
        let parser = DslParser;
        let input = "::button[label: Hello World ;action: say hello ;value: greeting ]";
        let result = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        match &result.instructions[0] {
            DslInstruction::Button {
                label,
                action,
                value,
            } => {
                assert_eq!(label, "Hello World");
                assert_eq!(action, "say hello");
                assert_eq!(value, "greeting");
            }
        }
    }

    #[test]
    fn test_multiple_dsl_with_text_scattered() {
        let parser = DslParser;
        let input = concat!(
            "::button[label:A;action:1;value:x]\n",
            "Text A\n",
            "::button[label:B;action:2;value:y]\n",
            "Text B\n",
            "::button[label:C;action:3;value:z]",
        );
        let result = parser.parse(input);

        assert_eq!(result.instructions.len(), 3);
        assert_eq!(result.clean_content, "Text A\nText B");
    }
}
