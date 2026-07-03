//! VerbosityFilter — outbound [`MessageProcessor`] for filtering content blocks
//! by session verbosity level.
//!
//! Reads `verbosity_level` from [`MessageContext`] metadata (injected by Gateway)
//! and filters `content_blocks` accordingly:
//! - [`VerbosityLevel::Full`]: no filtering
//! - [`VerbosityLevel::Normal`]: remove [`ContentBlock::Thinking`] blocks
//! - [`VerbosityLevel::Off`]: only keep [`ContentBlock::Text`] blocks
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

    /// Parse verbosity level from metadata string, defaulting to `Full`.
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
