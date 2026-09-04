//! Raw message logger processor.
//!
//! Writes incoming [`NormalizedMessage`] to a JSON file for audit and debugging purposes.
//!
//! # Conditional execution
//!
//! Log files are only written when [`RawLogConfig::enabled`] is `true`.
//! When disabled the processor silently skips writing and passes
//! the message through unchanged.
//!
use std::path::PathBuf;

use async_trait::async_trait;
use tokio::fs;

use super::context::MessageContext;
use super::error::ProcessError;
use super::processor::{MessageProcessor, ProcessPhase};
use super::ProcessedMessage;
use closeclaw_llm::types::ContentBlock;

/// Configuration for [`RawLogProcessor`].
#[derive(Debug, Clone)]
pub struct RawLogConfig {
    /// Whether to write log files regardless of log level.
    pub enabled: bool,
    /// Directory to write log files into. When `None`, the processor is disabled
    /// for writing even if `enabled` is `true` (no output destination).
    pub dir: Option<PathBuf>,
}

impl RawLogConfig {
    /// Creates a new config with the given values.
    pub fn new(enabled: bool, dir: Option<PathBuf>) -> Self {
        Self { enabled, dir }
    }

    /// Returns the log directory if configured, or an `InvalidData` error.
    pub fn require_dir(&self) -> std::io::Result<&PathBuf> {
        self.dir.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "raw_log dir not configured",
            )
        })
    }
}

/// Processor that writes raw messages to a JSON file.
#[derive(Debug)]
pub struct RawLogProcessor {
    config: RawLogConfig,
}

impl RawLogProcessor {
    /// Creates a new processor that writes to `config.dir`.
    pub fn new(config: RawLogConfig) -> Self {
        Self { config }
    }

    /// Writes `msg` to a JSON file under `self.config.dir`.
    ///
    /// Filename format: `{platform}_{timestamp_millis}.json`
    async fn write_log(
        &self,
        msg: &closeclaw_common::im_plugin::NormalizedMessage,
    ) -> std::io::Result<()> {
        let dir = self.config.require_dir()?;
        let filename = format!("{}_{}.json", msg.platform, msg.timestamp);
        let path = dir.join(&filename);

        let json = serde_json::to_string_pretty(msg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        fs::write(&path, json).await?;
        Ok(())
    }
}

#[async_trait]
impl MessageProcessor for RawLogProcessor {
    fn name(&self) -> &str {
        "raw_log"
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        10
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        let is_enabled = self.config.enabled && self.config.dir.is_some();
        if is_enabled {
            let raw = ctx.initial_normalized().ok_or_else(|| {
                ProcessError::invalid_message("no initial raw message in context")
            })?;

            self.write_log(raw)
                .await
                .map_err(|e| ProcessError::processor_failed(self.name(), e))?;
        }

        Ok(Some(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text(ctx.content.clone())],
            metadata: ctx.metadata.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use tempfile::TempDir;

    use super::*;
    use crate::processor_chain::context::MessageContext;
    use closeclaw_common::im_plugin::NormalizedMessage;

    fn make_normalized(platform: &str) -> NormalizedMessage {
        NormalizedMessage {
            platform: platform.to_string(),
            sender_id: "sender_1".to_string(),
            peer_id: String::new(),
            content: "hello".to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            reply_ref: None,
            unavailable_media: Vec::new(),
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
        }
    }

    fn make_ctx(msg: NormalizedMessage) -> MessageContext {
        MessageContext::from_normalized(msg)
    }

    #[tokio::test]
    async fn test_passes_through_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(false, Some(tmp.path().to_path_buf()));
        let processor = RawLogProcessor::new(config);

        let msg = make_normalized("feishu");
        let ctx = make_ctx(msg);

        let result = processor.process(&ctx).await.unwrap();
        let processed = result.expect("should pass through when disabled");
        assert_eq!(processed.content_blocks.len(), 1);
        match &processed.content_blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected Text block, got {other:?}"),
        }

        // disabled should not produce any log files
        let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        assert!(files.is_empty(), "no log files expected when disabled");
    }

    #[tokio::test]
    async fn test_enabled_but_no_dir_silently_skips() {
        let config = RawLogConfig::new(true, None);
        let processor = RawLogProcessor::new(config);

        let msg = make_normalized("feishu");
        let ctx = make_ctx(msg);

        // enabled=true but dir=None should behave like disabled: pass through, no error
        let result = processor.process(&ctx).await.unwrap();
        let processed = result.expect("should pass through when dir is None");
        assert_eq!(processed.content_blocks.len(), 1);
        match &processed.content_blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected Text block, got {other:?}"),
        }
    }

    #[test]
    fn test_require_dir_returns_error_when_none() {
        let config = RawLogConfig::new(true, None);
        let err = config.require_dir().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_require_dir_returns_path_when_some() {
        let config = RawLogConfig::new(true, Some("/tmp/test".into()));
        assert_eq!(config.require_dir().unwrap().to_str().unwrap(), "/tmp/test");
    }

    #[tokio::test]
    async fn test_write_file_when_enabled() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(true, Some(tmp.path().to_path_buf()));
        let processor = RawLogProcessor::new(config);

        let msg = make_normalized("feishu");
        let ctx = make_ctx(msg.clone());

        let result = processor.process(&ctx).await.unwrap();
        assert!(result.is_some());

        let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        let parsed: NormalizedMessage = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.platform, "feishu");
        assert_eq!(parsed.content, "hello");
    }

    #[tokio::test]
    async fn test_filename_format() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(true, Some(tmp.path().to_path_buf()));
        let processor = RawLogProcessor::new(config.clone());

        let ts = chrono::Utc::now().timestamp_millis();
        let msg = NormalizedMessage {
            platform: "wecom".to_string(),
            sender_id: "sender_1".to_string(),
            peer_id: String::new(),
            content: "hello".to_string(),
            timestamp: ts,
            message_type: Default::default(),
            media_refs: Vec::new(),
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
        };
        let ctx = make_ctx(msg);

        processor.process(&ctx).await.unwrap();

        let entries: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);

        let name = entries[0].file_name();
        let name_str = name.to_string_lossy();
        assert!(name_str.starts_with("wecom_"), "filename: {name_str}");
        assert!(name_str.ends_with(".json"), "filename: {name_str}");

        // filename format: {platform}_{timestamp_millis}.json
        let stem = Path::new(name_str.as_ref())
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        let parts: Vec<&str> = stem.splitn(2, '_').collect();
        assert_eq!(parts.len(), 2, "expected 2 segments: {stem}");
        assert_eq!(parts[0], "wecom");
        parts[1].parse::<i64>().unwrap();
    }

    #[tokio::test]
    async fn test_state_transition_disabled_then_enabled() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        // First call: disabled processor — should pass through, no log file
        let config_off = RawLogConfig::new(false, Some(dir.clone()));
        let processor_off = RawLogProcessor::new(config_off);
        let msg1 = make_normalized("feishu");
        let ctx1 = make_ctx(msg1);
        let result1 = processor_off.process(&ctx1).await.unwrap();
        assert!(result1.is_some(), "disabled should still pass through");
        let files_after_off: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert!(files_after_off.is_empty(), "no log files when disabled");

        // Second call: enabled processor — should write log and pass through
        let config_on = RawLogConfig::new(true, Some(dir.clone()));
        let processor_on = RawLogProcessor::new(config_on);
        let msg2 = make_normalized("feishu");
        let ctx2 = make_ctx(msg2);
        let result2 = processor_on.process(&ctx2).await.unwrap();
        assert!(result2.is_some(), "enabled should still pass through");
        let files_after_on: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(files_after_on.len(), 1, "one log file expected");
    }

    #[tokio::test]
    async fn test_state_transition_enabled_then_disabled() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        // First call: enabled — write log
        let config_on = RawLogConfig::new(true, Some(dir.clone()));
        let processor_on = RawLogProcessor::new(config_on);
        let msg1 = make_normalized("wecom");
        let ctx1 = make_ctx(msg1);
        let result1 = processor_on.process(&ctx1).await.unwrap();
        assert!(result1.is_some());
        let files_after_on: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(files_after_on.len(), 1);

        // Second call: disabled — pass through only
        let config_off = RawLogConfig::new(false, Some(dir.clone()));
        let processor_off = RawLogProcessor::new(config_off);
        let msg2 = make_normalized("wecom");
        let ctx2 = make_ctx(msg2);
        let result2 = processor_off.process(&ctx2).await.unwrap();
        assert!(result2.is_some(), "disabled should pass through");
        let files_after_off: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(
            files_after_off.len(),
            1,
            "still only one log file from the enabled call"
        );
    }

    #[tokio::test]
    async fn test_write_log_error_propagates() {
        // Use a non-existent subdirectory so write_log fails with ENOENT
        let base = TempDir::new().unwrap();
        let bad_dir = base.path().join("nonexistent_subdir");
        let config = RawLogConfig::new(true, Some(bad_dir));
        let processor = RawLogProcessor::new(config);

        let msg = make_normalized("feishu");
        let ctx = make_ctx(msg);

        let err = processor.process(&ctx).await.unwrap_err();
        let err_str = format!("{err}");
        assert!(
            err_str.contains("processor_failed") || err_str.contains("raw_log"),
            "expected processor error from write_log failure: {err_str}"
        );
    }
}
