//! RawLogProcessor — inbound processor that writes raw webhook JSON to files.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::Value;
use tracing::{debug, error, level_enabled};

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase, ProcessedMessage};

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

/// Processor that writes raw webhook JSON to a file for auditing.
#[derive(Debug)]
pub struct RawLogProcessor {
    config: RawLogConfig,
}

impl RawLogProcessor {
    /// Creates a new processor, cleaning up expired logs on startup.
    pub fn new(config: RawLogConfig) -> std::io::Result<Self> {
        let processor = Self { config };
        processor.retain_logs()?;
        Ok(processor)
    }

    /// Deletes log files whose embedded timestamp is older than retention_days.
    fn retain_logs(&self) -> std::io::Result<()> {
        let dir = &self.config.dir;
        if !dir.is_dir() {
            return Ok(());
        }
        let cutoff = Self::cutoff_secs(self.config.retention_days);
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(age) = parse_file_timestamp(&path) {
                    if age < cutoff {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
        Ok(())
    }

    /// Returns the cutoff timestamp in seconds since epoch.
    fn cutoff_secs(retention_days: u32) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX epoch");
        let retention_secs = (retention_days as u64) * 86400;
        (now.as_secs() - retention_secs) as i64
    }
}

/// Extracts the platform from raw JSON `channel` field, defaults to "feishu".
fn extract_platform(raw: &Value) -> String {
    raw.get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("feishu")
        .to_string()
}

/// Extracts the timestamp millis from `message.create_time`, falls back to now.
fn extract_timestamp_ms(raw: &Value) -> i64 {
    raw.get("message")
        .and_then(|m| m.get("create_time"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(current_timestamp_ms)
}

/// Returns current time in milliseconds since UNIX epoch.
fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis() as i64
}

/// Extracts the message_id from raw JSON `message.message_id`.
fn extract_message_id(raw: &Value) -> String {
    raw.get("message")
        .and_then(|m| m.get("message_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Extracts the content string from raw JSON `message.content`.
fn extract_content(raw: &Value) -> String {
    raw.get("message")
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Parses the timestamp from a log filename in milliseconds since epoch.
///
/// Expected format: `{platform}_{timestamp_millis}_{message_id}.json`
fn parse_file_timestamp(path: &Path) -> Option<i64> {
    let name = path.file_stem()?.to_str()?;
    let segments: Vec<&str> = name.split('_').collect();
    if segments.len() >= 2 {
        segments[1].parse::<i64>().ok().map(|ms| ms / 1000)
    } else {
        None
    }
}

#[async_trait]
impl MessageProcessor for RawLogProcessor {
    fn priority(&self) -> i32 {
        10
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    async fn process(
        &self,
        ctx: &MessageContext,
        msg: &Value,
    ) -> Result<ProcessedMessage, ProcessError> {
        let is_enabled = self.config.enabled || level_enabled!(tracing::Level::DEBUG);
        if !is_enabled {
            debug!("raw_log disabled, bypassing");
            return passthrough(msg, ctx);
        }

        let raw_webhook = match ctx.metadata.get("_raw_webhook") {
            Some(wh) => wh,
            None => {
                error!("no _raw_webhook in context, bypassing");
                return passthrough(msg, ctx);
            }
        };

        let raw: Value = match serde_json::from_str(raw_webhook) {
            Ok(v) => v,
            Err(e) => {
                error!("failed to parse _raw_webhook: {e}");
                return passthrough(msg, ctx);
            }
        };

        write_log_file(&self.config.dir, &raw).await;

        let content = extract_content(&raw);
        Ok(ProcessedMessage {
            content,
            metadata: ctx.metadata.clone(),
        })
    }
}

/// Writes the raw JSON to a file under the configured directory.
async fn write_log_file(dir: &Path, raw: &Value) {
    let platform = extract_platform(raw);
    let ts = extract_timestamp_ms(raw);
    let msg_id = extract_message_id(raw);
    let filename = format!("{platform}_{ts}_{msg_id}.json");
    let path = dir.join(&filename);

    let json = match serde_json::to_string_pretty(raw) {
        Ok(j) => j,
        Err(e) => {
            error!("failed to serialize raw log: {e}");
            return;
        }
    };

    if let Err(e) = tokio::fs::write(&path, json).await {
        error!("failed to write raw log file {}: {e}", path.display());
    }
}

/// Returns a passthrough result: original content + upstream metadata.
fn passthrough(msg: &Value, ctx: &MessageContext) -> Result<ProcessedMessage, ProcessError> {
    let content = msg
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(ProcessedMessage {
        content,
        metadata: ctx.metadata.clone(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn test_config(dir: &Path) -> RawLogConfig {
        RawLogConfig {
            enabled: true,
            dir: dir.to_path_buf(),
            retention_days: 7,
        }
    }

    fn raw_webhook_fixture() -> Value {
        serde_json::json!({
            "channel": "wecom",
            "message": {
                "message_id": "msg_abc123",
                "create_time": "1717737600000",
                "content": "{\"text\":\"hello\"}"
            }
        })
    }

    fn ctx_with_raw_webhook(raw: &Value) -> MessageContext {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "_raw_webhook".to_string(),
            serde_json::to_string(raw).unwrap(),
        );
        MessageContext { metadata }
    }

    #[tokio::test]
    async fn test_bypass_when_disabled_and_no_debug() {
        let tmp = TempDir::new().unwrap();
        let config = RawLogConfig {
            enabled: false,
            dir: tmp.path().to_path_buf(),
            retention_days: 7,
        };
        let processor = RawLogProcessor::new(config).unwrap();
        let ctx = MessageContext::default();
        let msg = raw_webhook_fixture();

        let result = processor.process(&ctx, &msg).await.unwrap();
        assert_eq!(result.content, "{\"text\":\"hello\"}");
        assert!(tmp.path().read_dir().unwrap().next().is_none());
    }

    #[tokio::test]
    async fn test_write_file_when_enabled() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path());
        let processor = RawLogProcessor::new(config).unwrap();
        let raw = raw_webhook_fixture();
        let ctx = ctx_with_raw_webhook(&raw);

        let result = processor.process(&ctx, &raw).await.unwrap();
        assert_eq!(result.content, "{\"text\":\"hello\"}");

        let files: Vec<_> = tmp.path().read_dir().unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
    }

    #[tokio::test]
    async fn test_filename_format() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path());
        let processor = RawLogProcessor::new(config).unwrap();
        let raw = raw_webhook_fixture();
        let ctx = ctx_with_raw_webhook(&raw);

        processor.process(&ctx, &raw).await.unwrap();

        let entries: Vec<_> = tmp.path().read_dir().unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let name = entries[0].file_name().to_string_lossy().into_owned();
        assert!(
            name.starts_with("wecom_"),
            "filename should start with platform: {name}"
        );
        assert!(
            name.ends_with("_msg_abc123.json"),
            "filename should end with _msg_id.json: {name}"
        );
        let stem = Path::new(name.as_str())
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        let parts: Vec<&str> = stem.splitn(3, '_').collect();
        assert_eq!(parts.len(), 3, "expected 3 segments: {stem}");
        assert_eq!(parts[0], "wecom");
        assert_eq!(parts[2], "msg_abc123");
        parts[1].parse::<i64>().unwrap();
    }

    #[tokio::test]
    async fn test_platform_defaults_to_feishu() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path());
        let processor = RawLogProcessor::new(config).unwrap();
        let raw = serde_json::json!({
            "message": {
                "message_id": "msg_001",
                "content": "{}"
            }
        });
        let ctx = ctx_with_raw_webhook(&raw);

        processor.process(&ctx, &raw).await.unwrap();

        let entries: Vec<_> = tmp.path().read_dir().unwrap().flatten().collect();
        let name = entries[0].file_name().to_string_lossy().into_owned();
        assert!(name.starts_with("feishu_"), "default platform: {name}");
    }

    #[tokio::test]
    async fn test_retain_logs_deletes_old_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let stale_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 100 * 86400;
        let stale_ms = (stale_secs * 1000) as i64;
        let stale = format!("feishu_{stale_ms}_stale_msg.json");
        std::fs::write(dir.join(&stale), "{}").unwrap();

        let fresh_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 86400;
        let fresh_ms = (fresh_secs * 1000) as i64;
        let fresh = format!("feishu_{fresh_ms}_fresh_msg.json");
        std::fs::write(dir.join(&fresh), "{}").unwrap();

        let config = test_config(dir);
        let _processor = RawLogProcessor::new(config).unwrap();

        let names: Vec<String> = dir
            .read_dir()
            .unwrap()
            .flatten()
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

    #[tokio::test]
    async fn test_priority_and_phase() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path());
        let processor = RawLogProcessor::new(config).unwrap();
        assert_eq!(processor.priority(), 10);
        assert_eq!(processor.phase(), ProcessPhase::Inbound);
    }

    #[tokio::test]
    async fn test_preserves_metadata() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path());
        let processor = RawLogProcessor::new(config).unwrap();
        let raw = raw_webhook_fixture();
        let mut ctx = ctx_with_raw_webhook(&raw);
        ctx.metadata
            .insert("chat_type".to_string(), "group".to_string());

        let result = processor.process(&ctx, &raw).await.unwrap();
        assert_eq!(result.metadata.get("chat_type").unwrap(), "group");
    }

    /// Fail-open: write failure must not block the message flow.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_write_failure_does_not_block_message() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        // Make the directory immutable so that file creation inside it
        // will fail with EPERM, exercising the fail-open error path.
        let is_root = unsafe { libc::getuid() == 0 };
        if is_root {
            let _ = std::process::Command::new("chattr")
                .args(["+i", &dir.to_string_lossy()])
                .output();
        } else {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555));
        }

        let config = test_config(dir);
        let processor = RawLogProcessor::new(config).unwrap();
        let raw = raw_webhook_fixture();
        let ctx = ctx_with_raw_webhook(&raw);

        // process() must succeed (fail-open) even when the write fails.
        let result = processor.process(&ctx, &raw).await.unwrap();
        assert_eq!(result.content, "{\"text\":\"hello\"}");

        // Clean up: remove immutable flag.
        if is_root {
            let _ = std::process::Command::new("chattr")
                .args(["-i", &dir.to_string_lossy()])
                .output();
        }
    }
}
