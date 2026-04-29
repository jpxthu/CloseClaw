//! Processor registry — holds inbound/outbound processor chains and drives execution.

use std::sync::Arc;

use super::context::{MessageContext, ProcessedMessage, RawMessage};
use super::error::ProcessError;
use super::processor::{MessageProcessor, ProcessPhase};

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

    /// Drives the inbound processor chain on `raw`.
    ///
    /// Processors are sorted by ascending [`priority`](MessageProcessor::priority) before
    /// execution. When a processor returns `Ok(Some(msg))` its result becomes the input for
    /// the next processor. If the chain is empty the `raw` message is converted directly to
    /// a [`ProcessedMessage`] (bypass).
    pub async fn process_inbound(&self, raw: RawMessage) -> Result<ProcessedMessage, ProcessError> {
        if self.inbound.is_empty() {
            return Ok(ProcessedMessage::from_raw(raw));
        }

        let mut ctx = MessageContext::from_raw(raw);

        let mut sorted = self.inbound.clone();
        sorted.sort_by_key(|p| p.priority());

        for processor in sorted {
            if ctx.skip {
                break;
            }
            match processor.process(&ctx).await? {
                Some(out) => {
                    ctx.content = out.content;
                    for (k, v) in out.metadata {
                        ctx.metadata.insert(k, v);
                    }
                    if out.suppress {
                        ctx.skip = true;
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
            content: ctx.content,
            metadata: ctx.metadata,
            suppress: ctx.skip,
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

        // Build a synthetic RawMessage so we can reuse MessageContext::from_raw.
        let synthetic_raw = RawMessage {
            platform: String::new(),
            sender_id: String::new(),
            content: llm_output.content.clone(),
            timestamp: chrono::Utc::now(),
            message_id: String::new(),
        };
        let mut ctx = MessageContext::from_raw(synthetic_raw);
        ctx.metadata = llm_output.metadata.clone();
        if llm_output.suppress {
            ctx.skip = true;
        }

        let mut sorted = self.outbound.clone();
        sorted.sort_by_key(|p| p.priority());

        for processor in sorted {
            if ctx.skip {
                break;
            }
            match processor.process(&ctx).await? {
                Some(out) => {
                    ctx.content = out.content;
                    for (k, v) in out.metadata {
                        ctx.metadata.insert(k, v);
                    }
                    if out.suppress {
                        ctx.skip = true;
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
            content: ctx.content,
            metadata: ctx.metadata,
            suppress: ctx.skip,
        })
    }
}
