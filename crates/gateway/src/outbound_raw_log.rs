//! Gateway-local seam for the simplified outbound raw-log write.
//!
//! The simplified outbound path (`send_outbound_simplified` /
//! `process_outbound_raw_log_only`) must write one outbound raw-log snapshot
//! and then forward the message **without** running VerbosityFilter /
//! DslParser / the middleware chain. The Gateway only owns the abstraction
//! here; the composition root (daemon / root crate) assembles the concrete
//! writer and injects it at construction time.
//!
//! `None` (bypass path, or a Gateway built without the injection) skips the
//! write entirely — the message is forwarded unchanged either way.

use std::collections::HashMap;

use async_trait::async_trait;
use closeclaw_common::processor::ContentBlock;

/// Snapshot handed to the injected outbound raw-log writer.
#[derive(Debug, Clone)]
pub struct OutboundRawLogSnapshot {
    /// Final message content — the simplified path's raw output.
    pub content: String,
    /// Structured content blocks carried by the message.
    pub content_blocks: Vec<ContentBlock>,
    /// Processor metadata (carries `channel`, `message_id`, ...).
    pub metadata: HashMap<String, String>,
}

/// Gateway-local abstraction over the simplified-path outbound raw-log write.
///
/// Implemented on the composition-root side; the Gateway never references the
/// concrete raw-log processor.
#[async_trait]
pub trait OutboundRawLogWriter: Send + Sync {
    /// Write one outbound snapshot.
    ///
    /// `Err` is fail-open: the Gateway logs the failure and forwards the
    /// original content unchanged, so a raw-log problem can never block a
    /// message.
    async fn write(&self, snapshot: OutboundRawLogSnapshot) -> Result<(), String>;
}
