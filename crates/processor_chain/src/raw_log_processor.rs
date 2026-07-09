//! Raw message logger processor.
//!
//! Writes incoming [`NormalizedMessage`] to a JSON file for audit and debugging purposes.
//!
//! # Conditional execution
//!
//! Log files are only written when **either** of the following conditions is met:
//!
//! - [`RawLogConfig::enabled`] is `true`, **or**
//! - the current log level is `DEBUG` or more verbose (`level_enabled!(tracing::Level::DEBUG)`).
//!
//! When neither condition holds the processor silently skips writing and passes
//! the message through unchanged. This design keeps production logging quiet by
//! default while allowing developers to opt-in via the `enabled` flag or by
//! setting the log level to `DEBUG`.
//!
//! # Lifecycle
//!
//! On construction ([`RawLogProcessor::new`]) stale log files older than
//! `config.retention_days` are automatically purged.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;
use tracing::level_enabled;

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
    /// Directory to write log files into.
    pub dir: PathBuf,
    /// Number of days to retain log files.
    pub retention_days: u32,
}

impl RawLogConfig {
    /// Creates a new config with the given values.
    pub fn new(enabled: bool, dir: PathBuf, retention_days: u32) -> Self {
        Self {
            enabled,
            dir,
            retention_days,
        }
    }
}

/// Processor that writes raw messages to a JSON file.
#[derive(Debug)]
pub struct RawLogProcessor {
    config: RawLogConfig,
}

impl RawLogProcessor {
    /// Creates a new processor that writes to `config.dir`.
    ///
    /// Old log files older than `config.retention_days` are deleted on startup.
    pub fn new(config: RawLogConfig) -> std::io::Result<Self> {
        let processor = Self { config };
        processor.retain_logs()?;
        Ok(processor)
    }

    /// Deletes log files whose embedded timestamp is older than `retention_days`.
    fn retain_logs(&self) -> std::io::Result<()> {
        let dir = &self.config.dir;
        if !dir.is_dir() {
            return Ok(());
        }

        let cutoff = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(self.config.retention_days as i64))
            .expect("retention_days out of range");
        let cutoff_secs = cutoff.timestamp();

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(age) = self.parse_file_timestamp(&path) {
                    if age < cutoff_secs {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
        Ok(())
    }

    /// Extracts the timestamp from a log filename.
    ///
    /// Expected format: `{platform}_{timestamp_millis}_{message_id}.json`
    fn parse_file_timestamp(&self, path: &Path) -> Option<i64> {
        let name = path.file_stem()?.to_str()?;
        let segments: Vec<&str> = name.split('_').collect();
        if segments.len() >= 2 {
            segments[1].parse::<i64>().ok().map(|ms| ms / 1000)
        } else {
            None
        }
    }

    /// Writes `msg` to a JSON file under `self.config.dir`.
    ///
    /// Filename format: `{platform}_{timestamp_millis}.json`
    async fn write_log(
        &self,
        msg: &closeclaw_common::im_plugin::NormalizedMessage,
    ) -> std::io::Result<()> {
        let filename = format!("{}_{}.json", msg.platform, msg.timestamp);
        let path = self.config.dir.join(&filename);

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
        let is_enabled = self.config.enabled || level_enabled!(tracing::Level::DEBUG);
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
            thread_id: None,
            account_id: String::new(),
        }
    }

    fn make_ctx(msg: NormalizedMessage) -> MessageContext {
        MessageContext::from_normalized(msg)
    }

    #[tokio::test]
    async fn test_bypass_when_disabled_and_no_debug() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(false, tmp.path().to_path_buf(), 7);
        let processor = RawLogProcessor::new(config).unwrap();

        let msg = make_normalized("feishu");
        let ctx = make_ctx(msg);

        let result = processor.process(&ctx).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_write_file_when_enabled() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
        let processor = RawLogProcessor::new(config).unwrap();

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
        let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
        let processor = RawLogProcessor::new(config.clone()).unwrap();

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
    async fn test_retain_logs_deletes_old_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let now_ms = chrono::Utc::now().timestamp_millis();

        // Write a "stale" file directly — timestamp in filename is 100 days ago
        let stale_ts = now_ms - 100 * 86_400_000;
        let stale_name = format!("feishu_{stale_ts}_stale_msg.json");
        std::fs::write(dir.join(&stale_name), "{}").unwrap();

        // Write a "fresh" file directly — timestamp in filename is 1 day ago
        let fresh_ts = now_ms - 1 * 86_400_000;
        let fresh_name = format!("feishu_{fresh_ts}_fresh_msg.json");
        std::fs::write(dir.join(&fresh_name), "{}").unwrap();

        let config = RawLogConfig::new(false, dir.clone(), 7);
        let _processor = RawLogProcessor::new(config).unwrap();

        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        let names: Vec<_> = files
            .iter()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("fresh_msg")),
            "fresh file should remain: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("stale_msg")),
            "stale file should be deleted: {names:?}"
        );
    }
}
