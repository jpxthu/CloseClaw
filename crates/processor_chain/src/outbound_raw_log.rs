//! Outbound raw message logger processor.
//!
//! Writes outbound [`MessageContext`] to a JSON file when enabled or in Debug
//! mode.  Mirrors [`super::raw_log_processor::RawLogProcessor`] but operates
//! on the outbound phase.

use async_trait::async_trait;
use tokio::fs;
use tracing::level_enabled;

use super::context::{MessageContext, ProcessedMessage};
use super::error::ProcessError;
use super::processor::{MessageProcessor, ProcessPhase};
use super::raw_log_processor::RawLogConfig;

/// Processor that writes outbound messages to a JSON log file.
///
/// The filename format is `{platform}_outbound_{timestamp_millis}_{message_id}.json`
/// to distinguish outbound logs from inbound logs produced by
/// [`super::raw_log_processor::RawLogProcessor`].
#[derive(Debug)]
pub struct OutboundRawLogProcessor {
    config: RawLogConfig,
}

impl OutboundRawLogProcessor {
    /// Creates a new outbound log processor.
    pub fn new(config: RawLogConfig) -> Self {
        Self { config }
    }

    /// Builds a serializable snapshot of the outbound context.
    fn build_snapshot(ctx: &MessageContext) -> OutboundSnapshot {
        OutboundSnapshot {
            content: ctx.content.clone(),
            content_blocks_summary: ctx
                .content_blocks
                .iter()
                .map(|b| format!("{:?}", b))
                .collect(),
            metadata: ctx.metadata.clone(),
        }
    }

    /// Writes the snapshot to a JSON file under `self.config.dir`.
    async fn write_log(&self, snapshot: &OutboundSnapshot) -> std::io::Result<()> {
        let timestamp_millis = chrono::Utc::now().timestamp_millis();
        let platform = snapshot
            .metadata
            .get("channel")
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let message_id = snapshot
            .metadata
            .get("message_id")
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("out-{}", timestamp_millis));
        let filename = format!(
            "{}_outbound_{}_{}.json",
            platform, timestamp_millis, message_id,
        );
        let path = self.config.dir.join(&filename);

        let json = serde_json::to_string_pretty(snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        fs::write(&path, json).await?;
        Ok(())
    }
}

/// Serializable snapshot of an outbound context for log files.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OutboundSnapshot {
    /// Final message content.
    content: String,
    /// Summaries of structured content blocks.
    content_blocks_summary: Vec<String>,
    /// Processor metadata.
    metadata: std::collections::HashMap<String, String>,
}

#[async_trait]
impl MessageProcessor for OutboundRawLogProcessor {
    fn name(&self) -> &str {
        "outbound_raw_log"
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Outbound
    }

    fn priority(&self) -> u8 {
        20
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        let is_enabled = self.config.enabled || level_enabled!(tracing::Level::DEBUG);
        if !is_enabled {
            return Ok(None);
        }

        let snapshot = Self::build_snapshot(ctx);

        self.write_log(&snapshot)
            .await
            .map_err(|e| ProcessError::processor_failed(self.name(), e))?;

        Ok(Some(ProcessedMessage {
            content_blocks: ctx.content_blocks.clone(),
            metadata: ctx.metadata.clone(),
        }))
    }
}

#[cfg(test)]
#[path = "outbound_raw_log_tests.rs"]
mod tests;
