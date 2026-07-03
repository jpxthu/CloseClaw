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
use std::collections::HashMap;
use tracing::warn;

use closeclaw_llm::types::ContentBlock;

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase};

// ---------------------------------------------------------------------------
// DSL data types
// ---------------------------------------------------------------------------

/// A parsed DSL instruction extracted from markdown.
///
/// Flat structure: `instruction_type` identifies the kind of instruction
/// (e.g. `"button"`, `"selector"`), and `params` holds key-value pairs
/// parsed from the DSL line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DslInstruction {
    /// Instruction type identifier (e.g. `"button"`, `"selector"`).
    pub instruction_type: String,
    /// Parsed key-value parameters from the DSL line.
    pub params: HashMap<String, String>,
}

/// Result of parsing a markdown string for DSL instructions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DslParseResult {
    /// Extracted DSL instructions in the order they appear in the source.
    pub instructions: Vec<DslInstruction>,
}

impl DslParseResult {
    /// Construct a [`DslParseResult`] from a slice of
    /// [`ContentBlock`][closeclaw_llm::types::ContentBlock].
    ///
    /// Only [`ContentBlock::Text`] variants are processed; [`ContentBlock::Thinking`],
    /// [`ContentBlock::ToolUse`], and [`ContentBlock::ToolResult`] are skipped.
    /// Internally delegates to [`DslParser::parse_content_blocks()`].
    pub fn from_content_blocks(blocks: &[ContentBlock]) -> Self {
        DslParser.parse_content_blocks(blocks)
    }
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
    pub fn parse(&self, content: &str) -> (DslParseResult, String) {
        let mut instructions: Vec<DslInstruction> = Vec::new();
        let mut clean_lines: Vec<&str> = Vec::new();

        for line in content.lines() {
            if let Some(instruction) = parse_dsl_line(line) {
                warn!(
                    instruction = ?instruction,
                    "DSL interaction type not supported by current renderer; skipping"
                );
                instructions.push(instruction);
            } else {
                clean_lines.push(line);
            }
        }

        let clean_text = clean_lines.join("\n");
        (DslParseResult { instructions }, clean_text)
    }

    /// Parse DSL instructions from a list of [`ContentBlock`][closeclaw_llm::types::ContentBlock].
    ///
    /// Only [`ContentBlock::Text`] variants are processed; [`ContentBlock::Thinking`],
    /// [`ContentBlock::ToolUse`], and [`ContentBlock::ToolResult`] are skipped.
    /// All text contents are concatenated with newlines before parsing.
    pub fn parse_content_blocks(&self, blocks: &[ContentBlock]) -> DslParseResult {
        let text: String = blocks
            .iter()
            .filter_map(|b| {
                if let ContentBlock::Text(s) = b {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let (result, _clean) = self.parse(&text);
        result
    }

    /// Parse DSL from `ContentBlock` list, returning both the merged [`DslParseResult`]
    /// and an updated `Vec<ContentBlock>` where Text blocks have DSL lines stripped
    /// and non-Text blocks are preserved as-is.
    ///
    /// Each Text block is processed independently so original block boundaries are
    /// retained. Empty Text blocks (after DSL stripping) are dropped.
    pub fn parse_content_blocks_with_result(
        &self,
        blocks: &[ContentBlock],
    ) -> (DslParseResult, Vec<ContentBlock>) {
        let mut all_instructions: Vec<DslInstruction> = Vec::new();
        let mut updated_blocks: Vec<ContentBlock> = Vec::new();

        for block in blocks {
            match block {
                ContentBlock::Text(s) => {
                    let (result, clean_text) = self.parse(s);
                    all_instructions.extend(result.instructions);
                    if !clean_text.is_empty() {
                        updated_blocks.push(ContentBlock::Text(clean_text));
                    }
                }
                _ => {
                    updated_blocks.push(block.clone());
                }
            }
        }

        (
            DslParseResult {
                instructions: all_instructions,
            },
            updated_blocks,
        )
    }
}

/// Try to parse a single line as a DSL instruction.
///
/// Returns `None` if the line is not a DSL line.
fn parse_dsl_line(line: &str) -> Option<DslInstruction> {
    let trimmed = line.trim();
    if !trimmed.ends_with(']') {
        return None;
    }

    if trimmed.starts_with("::button[") {
        return parse_button(trimmed);
    }
    if trimmed.starts_with("::selector[") {
        return parse_selector(trimmed);
    }

    None
}

/// Extract the bracket content from a DSL line.
///
/// Given `::tag[...]`, returns the trimmed inner content between `[` and `]`.
/// Returns `None` if brackets are empty or missing.
fn extract_bracket_content(trimmed: &str) -> Option<&str> {
    let start = trimmed.find('[')? + 1;
    let end = trimmed.len() - 1;
    if start >= end {
        return None;
    }
    Some(&trimmed[start..end])
}

/// Parse a `::button[...]` line into a [`DslInstruction`] with type "button".
fn parse_button(trimmed: &str) -> Option<DslInstruction> {
    let inner = extract_bracket_content(trimmed)?;

    let mut params = HashMap::new();

    for param in inner.split(';') {
        let param = param.trim();
        if let Some((key, val)) = param.split_once(':') {
            let key = key.trim();
            let val = val.trim();
            params.insert(key.to_string(), val.to_string());
        }
    }

    // label and action are required
    if !params.contains_key("label") || !params.contains_key("action") {
        return None;
    }
    // default value to empty string if not provided
    params.entry("value".to_string()).or_default();

    Some(DslInstruction {
        instruction_type: "button".to_string(),
        params,
    })
}

/// Parse a `::selector[...]` line into a [`DslInstruction`] with type "selector".
fn parse_selector(trimmed: &str) -> Option<DslInstruction> {
    let inner = extract_bracket_content(trimmed)?;

    let mut params = HashMap::new();

    for param in inner.split(';') {
        let param = param.trim();
        if let Some((key, val)) = param.split_once(':') {
            let key = key.trim();
            let val = val.trim();
            params.insert(key.to_string(), val.to_string());
        }
    }

    // label and action are required
    if !params.contains_key("label") || !params.contains_key("action") {
        return None;
    }

    Some(DslInstruction {
        instruction_type: "selector".to_string(),
        params,
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
        let (result, updated_blocks) = if !ctx.content_blocks.is_empty() {
            self.parse_content_blocks_with_result(&ctx.content_blocks)
        } else {
            let (result, clean_text) = self.parse(&ctx.content);
            let blocks = if clean_text.is_empty() {
                vec![]
            } else {
                vec![ContentBlock::Text(clean_text)]
            };
            (result, blocks)
        };

        let json = serde_json::to_string(&result)
            .map_err(|e| ProcessError::processor_failed("DslParser", e))?;

        let mut metadata = ctx.metadata.clone();
        metadata.insert("dsl_result".to_string(), json);

        Ok(Some(super::ProcessedMessage {
            content_blocks: if updated_blocks.is_empty() {
                vec![ContentBlock::Text(ctx.content.clone())]
            } else {
                updated_blocks
            },
            metadata,
        }))
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_llm::types::ContentBlock;

    #[test]
    fn test_no_dsl() {
        let parser = DslParser;
        let input = "Hello, this is a normal message without any DSL.";
        let (result, _clean) = parser.parse(input);

        assert!(result.instructions.is_empty());
    }

    #[test]
    fn test_single_dsl() {
        let parser = DslParser;
        let input = "::button[label:Click Me;action:navigate;value:/home]";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(
            result.instructions[0],
            DslInstruction {
                instruction_type: "button".to_string(),
                params: HashMap::from([
                    ("label".to_string(), "Click Me".to_string()),
                    ("action".to_string(), "navigate".to_string()),
                    ("value".to_string(), "/home".to_string()),
                ]),
            }
        );
    }

    #[test]
    fn test_multiple_dsl() {
        let parser = DslParser;
        let input =
            "::button[label:Yes;action:confirm;value:1]\n::button[label:No;action:cancel;value:0]";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 2);
        assert_eq!(result.instructions[0].instruction_type, "button");
        assert_eq!(result.instructions[0].params["label"], "Yes");
        assert_eq!(result.instructions[1].instruction_type, "button");
        assert_eq!(result.instructions[1].params["label"], "No");
    }

    #[test]
    fn test_dsl_mixed_with_text() {
        let parser = DslParser;
        let input = "Hello world\n::button[label:OK;action:submit;value:yes]\nGoodbye";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
    }

    #[test]
    fn test_dsl_at_first_line() {
        let parser = DslParser;
        let input = "::button[label:Start;action:begin;value:]\nNow the content starts here.";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
    }

    #[test]
    fn test_dsl_at_middle() {
        let parser = DslParser;
        let input = "Before\n::button[label:Middle;action:go;value:x]\nAfter";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
    }

    #[test]
    fn test_dsl_at_last_line() {
        let parser = DslParser;
        let input = "Some text here\n::button[label:End;action:finish;value:done]";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
    }

    #[test]
    fn test_dsl_param_with_spaces() {
        let parser = DslParser;
        let input = "::button[label: Hello World ;action: say hello ;value: greeting ]";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.instructions[0].params["label"], "Hello World");
        assert_eq!(result.instructions[0].params["action"], "say hello");
        assert_eq!(result.instructions[0].params["value"], "greeting");
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
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 3);
    }

    // ---------------------------------------------------------------------------
    // ContentBlock parse tests (Step 1.3)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_parse_content_blocks_empty() {
        let parser = DslParser;
        let blocks: Vec<ContentBlock> = vec![];
        let result = parser.parse_content_blocks(&blocks);
        assert!(result.instructions.is_empty());
    }

    #[test]
    fn test_parse_content_blocks_only_thinking() {
        let parser = DslParser;
        let blocks = vec![
            ContentBlock::Thinking {
                thinking: "Let me think about this...".to_string(),
                signature: None,
            },
            ContentBlock::Thinking {
                thinking: "Maybe I should try...".to_string(),
                signature: None,
            },
        ];
        let result = parser.parse_content_blocks(&blocks);
        assert!(result.instructions.is_empty());
    }

    #[test]
    fn test_parse_content_blocks_only_tool_use() {
        let parser = DslParser;
        let blocks = vec![
            ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "search".to_string(),
                input: "{}".to_string(),
            },
            ContentBlock::ToolUse {
                id: "call_2".to_string(),
                name: "fetch".to_string(),
                input: "{}".to_string(),
            },
        ];
        let result = parser.parse_content_blocks(&blocks);
        assert!(result.instructions.is_empty());
    }

    #[test]
    fn test_parse_content_blocks_only_tool_result() {
        let parser = DslParser;
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "call_1".to_string(),
            content: "some result".to_string(),
        }];
        let result = parser.parse_content_blocks(&blocks);
        assert!(result.instructions.is_empty());
    }

    #[test]
    fn test_parse_content_blocks_multiple_text_dsl_lines() {
        let parser = DslParser;
        let blocks = vec![
            ContentBlock::Text("Hello".to_string()),
            ContentBlock::Text("::button[label:A;action:x;value:1]".to_string()),
            ContentBlock::Text("Middle".to_string()),
            ContentBlock::Text("::button[label:B;action:y;value:2]".to_string()),
        ];
        let result = parser.parse_content_blocks(&blocks);
        assert_eq!(result.instructions.len(), 2);
    }

    #[test]
    fn test_parse_content_blocks_mixed_with_non_text_skipped() {
        let parser = DslParser;
        let blocks = vec![
            ContentBlock::Thinking {
                thinking: "thinking...".to_string(),
                signature: None,
            },
            ContentBlock::Text("::button[label:Click;action:go;value:ok]".to_string()),
            ContentBlock::ToolResult {
                tool_call_id: "call_1".to_string(),
                content: "tool result".to_string(),
            },
            ContentBlock::ToolUse {
                id: "call_2".to_string(),
                name: "test".to_string(),
                input: "{}".to_string(),
            },
        ];
        let result = parser.parse_content_blocks(&blocks);
        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.instructions[0].instruction_type, "button");
        assert_eq!(result.instructions[0].params["label"], "Click");
        assert_eq!(result.instructions[0].params["action"], "go");
        assert_eq!(result.instructions[0].params["value"], "ok");
    }

    #[test]
    fn test_from_content_blocks_equivalence() {
        let blocks = vec![
            ContentBlock::Text("Some text\n::button[label:X;action:a;value:1]".to_string()),
            ContentBlock::Thinking {
                thinking: "ignored".to_string(),
                signature: None,
            },
            ContentBlock::Text("More text\n::button[label:Y;action:b;value:2]".to_string()),
        ];
        let result_convenience = DslParseResult::from_content_blocks(&blocks);
        let result_manual = DslParser::default().parse_content_blocks(&blocks);
        assert_eq!(result_convenience, result_manual);
    }

    // -----------------------------------------------------------------------
    // Selector DSL tests (Step 1.3)
    // -----------------------------------------------------------------------

    #[test]
    fn test_single_selector() {
        let parser = DslParser;
        let input = "::selector[label:Pick color;options:Red,Green,Blue;action:select_color]";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(
            result.instructions[0],
            DslInstruction {
                instruction_type: "selector".to_string(),
                params: HashMap::from([
                    ("label".to_string(), "Pick color".to_string()),
                    ("options".to_string(), "Red,Green,Blue".to_string()),
                    ("action".to_string(), "select_color".to_string()),
                ]),
            }
        );
    }

    #[test]
    fn test_selector_empty_options() {
        let parser = DslParser;
        let input = "::selector[label:Choose;options:;action:pick]";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.instructions[0].params["label"], "Choose");
        assert_eq!(result.instructions[0].params["options"], "");
        assert_eq!(result.instructions[0].params["action"], "pick");
    }

    #[test]
    fn test_selector_with_spaces() {
        let parser = DslParser;
        let input = "::selector[label: Pick one ;options: A , B , C ;action: choose ]";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.instructions[0].params["label"], "Pick one");
        assert_eq!(result.instructions[0].params["options"], "A , B , C");
        assert_eq!(result.instructions[0].params["action"], "choose");
    }

    #[test]
    fn test_selector_mixed_with_text() {
        let parser = DslParser;
        let input = "Hello\n::selector[label:Pick;options:X,Y;action:go]\nWorld";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
    }

    #[test]
    fn test_selector_and_button_mixed() {
        let parser = DslParser;
        let input = concat!(
            "::button[label:Yes;action:confirm;value:1]\n",
            "::selector[label:Pick;options:A,B;action:choose]",
        );
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 2);
        assert_eq!(result.instructions[0].instruction_type, "button");
        assert_eq!(result.instructions[1].instruction_type, "selector");
    }

    #[test]
    fn test_selector_missing_label() {
        let parser = DslParser;
        let input = "::selector[options:A,B;action:go]";
        let (result, _clean) = parser.parse(input);

        assert!(result.instructions.is_empty());
    }

    #[test]
    fn test_selector_missing_action() {
        let parser = DslParser;
        let input = "::selector[label:Pick;options:A,B]";
        let (result, _clean) = parser.parse(input);

        assert!(result.instructions.is_empty());
    }

    #[test]
    fn test_selector_single_option() {
        let parser = DslParser;
        let input = "::selector[label:Only one;options:Only;action:single]";
        let (result, _clean) = parser.parse(input);

        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.instructions[0].params["options"], "Only");
    }
}
