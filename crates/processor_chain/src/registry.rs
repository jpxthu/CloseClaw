//! Processor registry — holds inbound/outbound processor chains and drives execution.

use std::sync::Arc;

use super::context::MessageContext;
use super::error::ProcessError;
use super::processor::{MessageProcessor, ProcessPhase};
use super::ProcessedMessage;
use async_trait::async_trait;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_llm::types::ContentBlock;

/// Registry holding inbound and outbound processor chains.
///
/// Processors are registered via [`register`](ProcessorRegistry::register) and
/// automatically routed to the appropriate chain based on their [`phase`](MessageProcessor::phase).
///
/// The two chains are driven independently by [`process_inbound`](ProcessorRegistry::process_inbound)
/// and [`process_outbound`](ProcessorRegistry::process_outbound).
#[derive(Default)]
pub struct ProcessorRegistry {
    inbound: Vec<Arc<dyn MessageProcessor>>,
    outbound: Vec<Arc<dyn MessageProcessor>>,
}

impl ProcessorRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            inbound: Vec::new(),
            outbound: Vec::new(),
        }
    }

    /// Registers a processor to the chain that matches its [`phase`](MessageProcessor::phase).
    pub fn register(&mut self, processor: Arc<dyn MessageProcessor>) -> &mut Self {
        match processor.phase() {
            ProcessPhase::Inbound => self.inbound.push(processor),
            ProcessPhase::Outbound => self.outbound.push(processor),
        }
        self
    }

    /// Returns the number of registered inbound processors.
    #[inline]
    pub fn inbound_len(&self) -> usize {
        self.inbound.len()
    }

    /// Returns the number of registered outbound processors.
    #[inline]
    pub fn outbound_len(&self) -> usize {
        self.outbound.len()
    }

    /// Drives the inbound processor chain on `msg`.
    ///
    /// Processors are sorted by ascending [`priority`](MessageProcessor::priority) before
    /// execution. When a processor returns `Ok(Some(msg))` its result becomes the input for
    /// the next processor. If the chain is empty the message content is converted directly to
    /// a [`ProcessedMessage`] (bypass).
    pub async fn process_inbound(
        &self,
        msg: NormalizedMessage,
    ) -> Result<ProcessedMessage, ProcessError> {
        if self.inbound.is_empty() {
            return Ok(ProcessedMessage::from_raw_content(msg.content));
        }

        let mut ctx = MessageContext::from_normalized(msg);

        let mut sorted = self.inbound.clone();
        sorted.sort_by_key(|p| p.priority());

        for processor in sorted {
            if ctx.skip {
                break;
            }
            match processor.process(&ctx).await? {
                Some(out) => {
                    ctx.content = out.text_content().unwrap_or("").to_string();
                    ctx.content_blocks = out.content_blocks;
                    for (k, v) in out.metadata {
                        ctx.metadata.insert(k, v);
                    }
                }
                None => {
                    // Processor chose to skip — halt the chain.
                    ctx.skip = true;
                    break;
                }
            }
        }

        Ok(ProcessedMessage {
            content_blocks: if ctx.skip {
                vec![]
            } else if ctx.content_blocks.is_empty() {
                vec![ContentBlock::Text(ctx.content)]
            } else {
                ctx.content_blocks
            },
            metadata: ctx.metadata,
        })
    }

    /// Drives the outbound processor chain on `llm_output`.
    ///
    /// Same semantics as [`process_inbound`](ProcessorRegistry::process_inbound) but operates on
    /// the outbound chain and takes a [`ProcessedMessage`] as input (converted internally to a
    /// [`MessageContext`]). If the chain is empty the input is returned unchanged (bypass).
    pub async fn process_outbound(
        &self,
        llm_output: ProcessedMessage,
    ) -> Result<ProcessedMessage, ProcessError> {
        if self.outbound.is_empty() {
            return Ok(llm_output);
        }

        // Build a synthetic NormalizedMessage so we can reuse MessageContext::from_normalized.
        let content = llm_output.text_content().unwrap_or("").to_string();
        let synthetic = NormalizedMessage {
            platform: String::new(),
            sender_id: String::new(),
            peer_id: String::new(),
            content,
            timestamp: chrono::Utc::now().timestamp_millis(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            thread_id: None,
            account_id: String::new(),
        };
        let mut ctx = MessageContext::from_normalized(synthetic);
        ctx.metadata = llm_output.metadata.clone();
        ctx.content_blocks = llm_output.content_blocks.clone();

        let mut sorted = self.outbound.clone();
        sorted.sort_by_key(|p| p.priority());

        for processor in sorted {
            if ctx.skip {
                break;
            }
            match processor.process(&ctx).await? {
                Some(out) => {
                    ctx.content = out.text_content().unwrap_or("").to_string();
                    ctx.content_blocks = out.content_blocks;
                    for (k, v) in out.metadata {
                        ctx.metadata.insert(k, v);
                    }
                }
                None => {
                    // Processor chose to skip — halt the chain.
                    ctx.skip = true;
                    break;
                }
            }
        }

        Ok(ProcessedMessage {
            content_blocks: if ctx.skip {
                vec![]
            } else if ctx.content_blocks.is_empty() {
                vec![ContentBlock::Text(ctx.content)]
            } else {
                ctx.content_blocks
            },
            metadata: ctx.metadata,
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// closeclaw_common::ProcessorChain impl
// ═══════════════════════════════════════════════════════════════════════════

fn convert_processed_message(m: ProcessedMessage) -> closeclaw_common::processor::ProcessedMessage {
    closeclaw_common::processor::ProcessedMessage {
        content_blocks: m.content_blocks,
        metadata: m.metadata,
    }
}

fn convert_process_error(e: ProcessError) -> closeclaw_common::processor::ProcessError {
    match e {
        ProcessError::ProcessorFailed { name, source } => {
            closeclaw_common::processor::ProcessError::ProcessorFailed { name, source }
        }
        ProcessError::InvalidMessage(s) => {
            closeclaw_common::processor::ProcessError::InvalidMessage(s)
        }
        ProcessError::ChainFailed(s) => closeclaw_common::processor::ProcessError::ChainFailed(s),
    }
}

#[async_trait]
impl closeclaw_common::processor::ProcessorChain for ProcessorRegistry {
    async fn process_inbound(
        &self,
        msg: NormalizedMessage,
    ) -> Result<
        closeclaw_common::processor::ProcessedMessage,
        closeclaw_common::processor::ProcessError,
    > {
        self.process_inbound(msg)
            .await
            .map(convert_processed_message)
            .map_err(convert_process_error)
    }

    async fn process_outbound(
        &self,
        msg: closeclaw_common::processor::ProcessedMessage,
    ) -> Result<
        closeclaw_common::processor::ProcessedMessage,
        closeclaw_common::processor::ProcessError,
    > {
        let main_msg = ProcessedMessage {
            content_blocks: msg.content_blocks,
            metadata: msg.metadata,
        };
        self.process_outbound(main_msg)
            .await
            .map(convert_processed_message)
            .map_err(convert_process_error)
    }

    fn inbound_len(&self) -> usize {
        self.inbound_len()
    }

    fn outbound_len(&self) -> usize {
        self.outbound_len()
    }
}
