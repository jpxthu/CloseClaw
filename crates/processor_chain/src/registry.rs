//! Processor registry — holds inbound/outbound processor chains and drives execution.

use std::sync::Arc;

use super::context::MessageContext;
use super::dsl_parser::DslParser;
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
/// The two chains are driven independently by
/// [`process_inbound`](ProcessorRegistry::process_inbound)
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
            match processor.process(&ctx).await {
                Ok(Some(out)) => {
                    ctx.content = out.text_content().unwrap_or("").to_string();
                    ctx.content_blocks = out.content_blocks;
                    for (k, v) in out.metadata {
                        ctx.metadata.insert(k, v);
                    }
                }
                Ok(None) => {
                    // Processor chose to skip — halt the chain.
                    ctx.skip = true;
                    break;
                }
                Err(e) => {
                    if processor.name() == "raw_log" {
                        tracing::error!(
                            processor = %processor.name(),
                            error = %e,
                            "processor failed, continuing chain"
                        );
                    } else {
                        tracing::warn!(
                            processor = %processor.name(),
                            error = %e,
                            "processor failed, continuing chain"
                        );
                    }
                    // Do not update ctx — skip this processor's result.
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
// Outbound chain helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Build a synthetic [`NormalizedMessage`] from a [`ProcessedMessage`] so we
/// can reuse [`MessageContext::from_normalized`] in the outbound chain.
fn synthetic_from_output(output: &ProcessedMessage) -> NormalizedMessage {
    NormalizedMessage {
        platform: String::new(),
        sender_id: String::new(),
        peer_id: String::new(),
        content: output.text_content().unwrap_or("").to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: String::new(),
        chat_name: String::new(),
        trace_id: String::new(),
        message_id: String::new(),
    }
}

impl ProcessorRegistry {
    /// Internal helper: drive the outbound chain with optional name filters.
    ///
    /// When `exclude` is non-empty, any processor whose name matches any entry
    /// is skipped. This allows running partial chains (e.g. skip DslParser +
    /// OutboundRawLog in the incremental phase, or skip VerbosityFilter in the
    /// DslParser-only path).
    async fn process_outbound_filtered(
        &self,
        llm_output: ProcessedMessage,
        exclude: &[&str],
    ) -> Result<ProcessedMessage, ProcessError> {
        if self.outbound.is_empty() {
            return Ok(llm_output);
        }

        let mut ctx = MessageContext::from_normalized(synthetic_from_output(&llm_output));
        ctx.metadata = llm_output.metadata.clone();
        ctx.content_blocks = llm_output.content_blocks.clone();
        let had_content_blocks = !ctx.content_blocks.is_empty();

        let mut sorted = self.outbound.clone();
        sorted.sort_by_key(|p| p.priority());

        for processor in sorted {
            if ctx.skip {
                break;
            }
            if exclude.contains(&processor.name()) {
                continue;
            }
            match processor.process(&ctx).await {
                Ok(Some(out)) => {
                    ctx.content = out.text_content().unwrap_or("").to_string();
                    ctx.content_blocks = out.content_blocks;
                    for (k, v) in out.metadata {
                        ctx.metadata.insert(k, v);
                    }
                }
                Ok(None) => {
                    // Processor chose to skip — halt the chain.
                    ctx.skip = true;
                    break;
                }
                Err(e) => {
                    if processor.name() == "raw_log" {
                        tracing::error!(
                            processor = %processor.name(),
                            error = %e,
                            "processor failed, continuing chain"
                        );
                    } else {
                        tracing::warn!(
                            processor = %processor.name(),
                            error = %e,
                            "processor failed, continuing chain"
                        );
                    }
                    // Do not update ctx — skip this processor's result.
                    // VerbosityFilter fail: keep previous content_blocks (Full behavior)
                    // DslParser fail: keep original Text blocks with DSL lines (passthrough)
                    // RawLog fail: skip log entry, continue sending
                }
            }
        }

        Ok(ProcessedMessage {
            content_blocks: if ctx.skip {
                vec![]
            } else if ctx.content_blocks.is_empty() && !had_content_blocks {
                // Only fall back to plain text when no content_blocks were
                // provided initially (empty chain / bypass path). When blocks
                // were provided but a processor (e.g. VerbosityFilter) removed
                // them all, we must return empty — not a spurious Text("").
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
        self.process_outbound_filtered(llm_output, &[]).await
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

    async fn process_outbound_without_verbosity(
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
        self.process_outbound_filtered(main_msg, &["verbosity_filter"])
            .await
            .map(convert_processed_message)
            .map_err(convert_process_error)
    }

    async fn process_outbound_incremental(
        &self,
        msg: closeclaw_common::processor::ProcessedMessage,
    ) -> Result<
        closeclaw_common::processor::ProcessedMessage,
        closeclaw_common::processor::ProcessError,
    > {
        // Incremental phase — explicit processor handling:
        // - VerbosityFilter (priority 5): normal execution
        // - DslParser (priority 10): passthrough mode via public API
        // - OutboundRawLog (priority 20): skipped
        if self.outbound.is_empty() {
            return Ok(msg);
        }

        let mut ctx = MessageContext::from_normalized(synthetic_from_output(&ProcessedMessage {
            content_blocks: msg.content_blocks.clone(),
            metadata: msg.metadata.clone(),
        }));
        ctx.metadata = msg.metadata;
        ctx.content_blocks = msg.content_blocks;
        let had_content_blocks = !ctx.content_blocks.is_empty();

        // 1. VerbosityFilter — normal execution
        let vf = self
            .outbound
            .iter()
            .find(|p| p.name() == "verbosity_filter");
        if let Some(processor) = vf {
            match processor.process(&ctx).await {
                Ok(Some(out)) => {
                    ctx.content = out.text_content().unwrap_or("").to_string();
                    ctx.content_blocks = out.content_blocks;
                    for (k, v) in out.metadata {
                        ctx.metadata.insert(k, v);
                    }
                }
                Ok(None) => {
                    ctx.skip = true;
                }
                Err(e) => {
                    tracing::warn!(
                        processor = %processor.name(),
                        error = %e,
                        "processor failed, continuing chain"
                    );
                }
            }
        }

        // 2. DslParser — passthrough mode (parse DSL, write metadata,
        //    but do NOT strip DSL lines from content blocks)
        if !ctx.skip {
            let dsl = self.outbound.iter().find(|p| p.name() == "DslParser");
            if dsl.is_some() {
                // Passthrough: parse DSL into metadata only.
                // parse_content_blocks reads Text blocks for DSL lines,
                // content blocks remain unchanged.
                let dsl_result = DslParser.parse_content_blocks(&ctx.content_blocks);
                if !dsl_result.instructions.is_empty() {
                    let json = serde_json::to_string(&dsl_result).map_err(|e| {
                        convert_process_error(ProcessError::processor_failed("DslParser", e))
                    })?;
                    ctx.metadata.insert("dsl_result".into(), json);
                }
            }
        }

        // 3. OutboundRawLog — skipped
        // 4. Build result
        let content_blocks = if ctx.skip {
            vec![]
        } else if ctx.content_blocks.is_empty() && !had_content_blocks {
            vec![ContentBlock::Text(ctx.content)]
        } else {
            ctx.content_blocks
        };
        Ok(convert_processed_message(ProcessedMessage {
            content_blocks,
            metadata: ctx.metadata,
        }))
    }

    fn inbound_len(&self) -> usize {
        self.inbound_len()
    }

    fn outbound_len(&self) -> usize {
        self.outbound_len()
    }

    async fn process_outbound_raw_log_only(
        &self,
        msg: closeclaw_common::processor::ProcessedMessage,
    ) -> Result<
        closeclaw_common::processor::ProcessedMessage,
        closeclaw_common::processor::ProcessError,
    > {
        // Find the OutboundRawLog processor (name == "outbound_raw_log").
        let raw_log = self
            .outbound
            .iter()
            .find(|p| p.name() == "outbound_raw_log")
            .cloned();
        let Some(processor) = raw_log else {
            // No OutboundRawLog registered — return input unchanged.
            return Ok(msg);
        };
        let mut ctx = super::context::MessageContext::from_normalized(
            closeclaw_common::im_plugin::NormalizedMessage {
                platform: String::new(),
                sender_id: String::new(),
                peer_id: String::new(),
                content: msg.text_content().unwrap_or("").to_string(),
                timestamp: chrono::Utc::now().timestamp_millis(),
                message_type: Default::default(),
                media_refs: Vec::new(),
                thread_id: None,
                account_id: String::new(),
                chat_name: String::new(),
                trace_id: String::new(),
                message_id: String::new(),
            },
        );
        ctx.metadata = msg.metadata.clone();
        ctx.content_blocks = msg.content_blocks.clone();
        match processor.process(&ctx).await {
            Ok(Some(out)) => Ok(convert_processed_message(out)),
            Ok(None) => Ok(msg),
            Err(e) => Err(convert_process_error(e)),
        }
    }

    fn parse_line_for_dsl(
        &self,
        line: &str,
    ) -> (String, closeclaw_common::processor::DslParseResult) {
        let (dsl_result, clean_text) = DslParser.parse(line);
        (clean_text, dsl_result)
    }
}
