//! Composition-root implementation of the Gateway's outbound raw-log seam.
//!
//! The Gateway consumes `closeclaw_gateway::outbound_raw_log` only; the
//! concrete writer assembled here delegates to the processor chain's
//! [`OutboundRawLogProcessor`], so the snapshot layout, filename scheme and
//! fail-open semantics are byte-for-byte the pre-migration ones.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_gateway::outbound_raw_log::{OutboundRawLogSnapshot, OutboundRawLogWriter};
use closeclaw_gateway::GatewayConfig;
use closeclaw_processor_chain::context::MessageContext;
use closeclaw_processor_chain::outbound_raw_log::OutboundRawLogProcessor;
use closeclaw_processor_chain::processor::MessageProcessor;
use closeclaw_processor_chain::raw_log_processor::RawLogConfig;

/// Writes simplified-path outbound snapshots into `dir` through the
/// processor chain's [`OutboundRawLogProcessor`].
pub struct ChainOutboundRawLogWriter {
    /// Target directory (`GatewayConfig::raw_log_dir`).
    dir: PathBuf,
}

impl ChainOutboundRawLogWriter {
    /// Create a writer that stores snapshots under `dir`.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

#[async_trait]
impl OutboundRawLogWriter for ChainOutboundRawLogWriter {
    async fn write(&self, snapshot: OutboundRawLogSnapshot) -> Result<(), String> {
        let ctx = MessageContext {
            content: snapshot.content,
            raw_message_log: Vec::new(),
            metadata: snapshot.metadata,
            skip: false,
            content_blocks: snapshot.content_blocks,
        };
        let processor =
            OutboundRawLogProcessor::new(RawLogConfig::new(true, Some(self.dir.clone())));
        match processor.process(&ctx).await {
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Convenience wrapper: the writer injected into `Gateway::new`.
///
/// Returns `None` when `raw_log_dir` is not configured, so the Gateway skips
/// the simplified-path write exactly as before the seam existed.
pub fn build_outbound_raw_log_writer(
    config: &GatewayConfig,
) -> Option<Arc<dyn OutboundRawLogWriter>> {
    config
        .raw_log_dir
        .clone()
        .map(|dir| Arc::new(ChainOutboundRawLogWriter::new(dir)) as Arc<dyn OutboundRawLogWriter>)
}
