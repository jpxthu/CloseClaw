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
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let message_id = snapshot
            .metadata
            .get("message_id")
            .and_then(|v| v.as_str())
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
    metadata: serde_json::Map<String, serde_json::Value>,
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
            content: ctx.content.clone(),
            metadata: ctx.metadata.clone(),
            suppress: false,
            content_blocks: ctx.content_blocks.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor_chain::context::{MessageContext, RawMessage};
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_ctx(content: &str, channel: &str, message_id: &str) -> MessageContext {
        let raw = RawMessage {
            platform: channel.to_string(),
            sender_id: "sender_1".to_string(),
            peer_id: String::new(),
            content: content.to_string(),
            timestamp: Utc::now(),
            message_id: message_id.to_string(),
        };
        let mut ctx = MessageContext::from_raw(raw);
        ctx.metadata.insert(
            "channel".to_string(),
            serde_json::Value::String(channel.to_string()),
        );
        ctx
    }

    #[tokio::test]
    async fn test_outbound_phase_and_priority() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(false, tmp.path().to_path_buf(), 7);
        let processor = OutboundRawLogProcessor::new(config);
        assert_eq!(processor.phase(), ProcessPhase::Outbound);
        assert_eq!(processor.priority(), 20);
        assert_eq!(processor.name(), "outbound_raw_log");
    }

    #[tokio::test]
    async fn test_bypass_when_disabled_and_no_debug() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(false, tmp.path().to_path_buf(), 7);
        let processor = OutboundRawLogProcessor::new(config);

        let ctx = make_ctx("hello", "terminal", "msg_1");
        let result = processor.process(&ctx).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_write_file_when_enabled() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
        let processor = OutboundRawLogProcessor::new(config);

        let ctx = make_ctx("hi there", "feishu", "msg_42");
        let result = processor.process(&ctx).await.unwrap();
        assert!(result.is_some());

        let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);

        let name = files[0].file_name();
        let name_str = name.to_string_lossy();
        assert!(
            name_str.contains("_outbound_"),
            "filename should contain _outbound_: {name_str}"
        );
        assert!(
            name_str.starts_with("feishu_outbound_"),
            "filename: {name_str}"
        );
        assert!(
            name_str.ends_with(".json"),
            "filename should end with .json: {name_str}"
        );
    }

    #[tokio::test]
    async fn test_write_file_with_message_id_metadata() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
        let processor = OutboundRawLogProcessor::new(config);

        let mut ctx = make_ctx("hi there", "feishu", "msg_42");
        ctx.metadata.insert(
            "message_id".to_string(),
            serde_json::Value::String("msg_42".to_string()),
        );
        let result = processor.process(&ctx).await.unwrap();
        assert!(result.is_some());

        let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);

        let name = files[0].file_name();
        let name_str = name.to_string_lossy();
        assert!(
            name_str.starts_with("feishu_outbound_"),
            "filename: {name_str}"
        );
        assert!(name_str.ends_with("_msg_42.json"), "filename: {name_str}");
    }

    #[tokio::test]
    async fn test_outbound_and_independent_from_inbound() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);

        let inbound =
            super::super::raw_log_processor::RawLogProcessor::new(config.clone()).unwrap();
        let outbound = OutboundRawLogProcessor::new(config);

        let raw = RawMessage {
            platform: "wecom".to_string(),
            sender_id: "s".to_string(),
            peer_id: String::new(),
            content: "hello".to_string(),
            timestamp: Utc::now(),
            message_id: "msg_99".to_string(),
        };
        let inbound_ctx = MessageContext::from_raw(raw.clone());
        inbound.process(&inbound_ctx).await.unwrap();

        let mut outbound_ctx = make_ctx("reply", "wecom", "msg_99");
        outbound_ctx.metadata.insert(
            "message_id".to_string(),
            serde_json::Value::String("msg_99".to_string()),
        );
        outbound.process(&outbound_ctx).await.unwrap();

        let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        assert_eq!(files.len(), 2);

        let names: Vec<_> = files
            .iter()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().any(|n| !n.contains("_outbound_")),
            "should have an inbound log: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("_outbound_")),
            "should have an outbound log: {names:?}"
        );
    }

    #[tokio::test]
    async fn test_preserves_content_and_blocks() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
        let processor = OutboundRawLogProcessor::new(config);

        let mut ctx = make_ctx("output text", "terminal", "msg_5");
        ctx.metadata.insert(
            "session_key".to_string(),
            serde_json::Value::String("sess_1".to_string()),
        );

        let result = processor.process(&ctx).await.unwrap().unwrap();
        assert_eq!(result.content, "output text");
        assert_eq!(
            result.metadata.get("session_key").and_then(|v| v.as_str()),
            Some("sess_1")
        );
        assert!(result.content_blocks.is_empty());
    }
}
