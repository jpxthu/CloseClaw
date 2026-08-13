//! VerbosityFilter — outbound [`MessageProcessor`] for filtering content blocks
//! by session verbosity level.
//!
//! Reads `verbosity_level` from [`MessageContext`] metadata (injected by Gateway)
//! and filters `content_blocks` accordingly:
//! - [`VerbosityLevel::Full`]: no filtering
//! - [`VerbosityLevel::Normal`]: remove [`ContentBlock::Thinking`] blocks
//! - [`VerbosityLevel::Off`]: only keep [`ContentBlock::Text`] blocks;
//!   all other block types are filtered out
//!
//! Priority 5 — runs before [`DslParser`] (priority 10).

use std::str::FromStr;

use async_trait::async_trait;

use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::ContentBlock;

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase};

/// Outbound processor that filters content blocks by verbosity level.
#[derive(Debug, Clone, Default)]
pub struct VerbosityFilter;

impl VerbosityFilter {
    /// Filter content blocks by the given verbosity level.
    pub fn filter(blocks: Vec<ContentBlock>, level: VerbosityLevel) -> Vec<ContentBlock> {
        match level {
            VerbosityLevel::Full => blocks,
            VerbosityLevel::Normal => blocks
                .into_iter()
                .filter(|b| !matches!(b, ContentBlock::Thinking { .. }))
                .collect(),
            VerbosityLevel::Off => blocks
                .into_iter()
                .filter(|b| matches!(b, ContentBlock::Text(_)))
                .collect(),
        }
    }

    /// Parse verbosity level from metadata string, defaulting to `Normal`.
    pub(crate) fn verbosity_from_metadata(
        metadata: &std::collections::HashMap<String, String>,
    ) -> VerbosityLevel {
        metadata
            .get("verbosity_level")
            .and_then(|v| VerbosityLevel::from_str(v).ok())
            .unwrap_or_default()
    }
}

#[async_trait]
impl MessageProcessor for VerbosityFilter {
    fn name(&self) -> &str {
        "verbosity_filter"
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Outbound
    }

    fn priority(&self) -> u8 {
        5
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<super::ProcessedMessage>, ProcessError> {
        let level = Self::verbosity_from_metadata(&ctx.metadata);

        let filtered = if ctx.content_blocks.is_empty() {
            // Fallback: filter the plain content string if no blocks.
            // This handles cases where content_blocks is not yet populated.
            vec![ContentBlock::Text(ctx.content.clone())]
        } else {
            Self::filter(ctx.content_blocks.clone(), level)
        };

        Ok(Some(super::ProcessedMessage {
            content_blocks: filtered,
            metadata: ctx.metadata.clone(),
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
    fn test_off_keeps_only_text_blocks() {
        let blocks = vec![
            ContentBlock::Text("hello".into()),
            ContentBlock::Image {
                name: "img.png".into(),
                url: "http://example.com/img.png".into(),
            },
            ContentBlock::Audio {
                name: "audio.wav".into(),
                url: "http://example.com/audio.wav".into(),
            },
            ContentBlock::File {
                name: "doc.pdf".into(),
                url: "http://example.com/doc.pdf".into(),
            },
            ContentBlock::Thinking {
                thinking: "reasoning".into(),
                signature: None,
            },
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "tool_a".into(),
                input: "{}".into(),
            },
            ContentBlock::ToolResult {
                tool_call_id: "t1".into(),
                content: "result".into(),
            },
        ];
        let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], ContentBlock::Text(t) if t == "hello"));
    }

    #[test]
    fn test_off_empty_input_returns_empty() {
        let result = VerbosityFilter::filter(vec![], VerbosityLevel::Off);
        assert!(result.is_empty());
    }

    #[test]
    fn test_normal_filters_thinking_only() {
        let blocks = vec![
            ContentBlock::Text("hello".into()),
            ContentBlock::Thinking {
                thinking: "reasoning".into(),
                signature: None,
            },
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "tool_a".into(),
                input: "{}".into(),
            },
        ];
        let result = VerbosityFilter::filter(blocks, VerbosityLevel::Normal);
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], ContentBlock::Text(_)));
        assert!(matches!(&result[1], ContentBlock::ToolUse { .. }));
    }

    #[test]
    fn test_full_preserves_all_blocks() {
        let blocks = vec![
            ContentBlock::Text("hello".into()),
            ContentBlock::Thinking {
                thinking: "reasoning".into(),
                signature: None,
            },
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "tool_a".into(),
                input: "{}".into(),
            },
            ContentBlock::ToolResult {
                tool_call_id: "t1".into(),
                content: "result".into(),
            },
        ];
        let result = VerbosityFilter::filter(blocks, VerbosityLevel::Full);
        assert_eq!(result.len(), 4);
    }
}
