//! Audit logging for dangerous operations in Auto Mode.
//!
//! Records structured logs for both approved and rejected permission requests.
//! Follows the same JSON Lines format and truncation logic as `RejectionLog`.

use super::engine_risk::RiskLevel;
use super::engine_types::PermissionRequestBody;
use closeclaw_common::session_mode::SessionMode;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::Mutex;

/// Disposition of an audited permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDisposition {
    /// The operation was approved by the user.
    Approved,
    /// The operation was rejected (by user or engine).
    Rejected,
}

impl std::fmt::Display for AuditDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditDisposition::Approved => write!(f, "approved"),
            AuditDisposition::Rejected => write!(f, "rejected"),
        }
    }
}

/// A single audit log entry for a permission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    /// Timestamp of the event (ISO 8601).
    pub timestamp: String,
    /// Agent ID involved.
    pub agent_id: String,
    /// Tool/request type name.
    pub tool_name: String,
    /// Operation description (e.g. "write", "read", command text).
    pub operation: String,
    /// Human-readable reason for the disposition.
    pub reason: String,
    /// Risk level of the operation.
    pub risk_level: RiskLevel,
    /// Session mode at the time of the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_mode: Option<SessionMode>,
    /// Final disposition of the operation.
    pub disposition: AuditDisposition,
}

/// Trait for recording audit log entries.
pub trait AuditLogger: Send + Sync {
    /// Log an audit entry.
    fn log(&self, entry: &AuditLogEntry);
}

/// File-based audit logger using JSON Lines format.
///
/// When constructed with [`FileAuditLogger::new_with_limit`], the logger
/// enforces a maximum number of entries by truncating old entries on write.
pub struct FileAuditLogger {
    path: PathBuf,
    max_entries: Option<usize>,
    writer: Mutex<()>,
}

impl FileAuditLogger {
    /// Create a new file logger that appends to the given path.
    /// Parent directories are created if they don't exist.
    pub fn new(path: PathBuf) -> std::io::Result<Self> {
        Self::new_with_limit(path, None)
    }

    /// Create a new file logger with a maximum entry limit.
    ///
    /// When `max_entries` is `Some(n)`, the logger ensures the log file
    /// never exceeds `n` lines. If the limit is reached, the oldest entries
    /// are truncated before appending the new entry.
    pub fn new_with_limit(
        path: PathBuf,
        max_entries: Option<usize>,
    ) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Ensure file exists
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Self {
            path,
            max_entries,
            writer: Mutex::new(()),
        })
    }

    /// Returns the path this logger writes to.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Returns the configured maximum entry limit, if any.
    pub fn max_entries(&self) -> Option<usize> {
        self.max_entries
    }

    /// Count non-empty lines in the log file.
    fn count_entries(path: &PathBuf) -> usize {
        std::fs::File::open(path)
            .map(|f| {
                io::BufReader::new(f)
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| !l.trim().is_empty())
                    .count()
            })
            .unwrap_or(0)
    }

    /// Truncate old entries, keeping the newest `keep` lines.
    /// Since entries are stored in reverse chronological order (newest first),
    /// this keeps the first `keep` lines.
    fn truncate_old_entries(path: &PathBuf, keep: usize) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let lines: Vec<&str> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        if lines.len() <= keep {
            return;
        }
        let kept: String =
            lines.iter().take(keep).map(|l| format!("{l}\n")).collect();
        let _ = std::fs::write(path, kept);
    }

    /// Write a single entry to the log file, prepending it so the newest
    /// entry is always at the top (reverse chronological order).
    fn write_entry(&self, entry: &AuditLogEntry) {
        let new_line = match serde_json::to_vec(entry) {
            Ok(mut line) => {
                line.push(b'\n');
                line
            }
            Err(_) => return,
        };
        let existing =
            std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut combined = new_line;
        combined.extend_from_slice(existing.as_bytes());
        let _ = std::fs::write(&self.path, combined);
    }
}

impl AuditLogger for FileAuditLogger {
    fn log(&self, entry: &AuditLogEntry) {
        let _lock = self.writer.lock();

        if let Some(max) = self.max_entries {
            let count = Self::count_entries(&self.path);
            if count >= max {
                Self::truncate_old_entries(&self.path, max - 1);
            }
        }

        self.write_entry(entry);
    }
}

impl std::fmt::Debug for FileAuditLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileAuditLogger")
            .field("path", &self.path)
            .field("max_entries", &self.max_entries)
            .finish()
    }
}

/// Build an [`AuditLogEntry`] from a request body and disposition metadata.
pub fn build_audit_log(
    body: &PermissionRequestBody,
    disposition: AuditDisposition,
    reason: String,
    risk_level: RiskLevel,
    session_mode: Option<SessionMode>,
) -> AuditLogEntry {
    let (tool_name, operation) = match body {
        PermissionRequestBody::FileOp { path, op, .. } => {
            ("file".to_string(), format!("{} {}", op, path))
        }
        PermissionRequestBody::CommandExec { cmd, args, .. } => (
            "command".to_string(),
            format!("{} {}", cmd, args.join(" ")),
        ),
        PermissionRequestBody::NetOp { host, port, .. } => {
            ("network".to_string(), format!("{}:{}", host, port))
        }
        PermissionRequestBody::ToolCall { skill, method, .. } => {
            ("tool_call".to_string(), format!("{}.{}", skill, method))
        }
        PermissionRequestBody::InterAgentMsg { to, .. } => {
            ("inter_agent".to_string(), format!("msg to {}", to))
        }
        PermissionRequestBody::ConfigWrite { config_file, .. } => {
            ("config_write".to_string(), config_file.clone())
        }
        PermissionRequestBody::SlashCommand { command, .. } => {
            ("slash_command".to_string(), command.clone())
        }
        PermissionRequestBody::MessageSend {
            direction,
            target,
            ..
        } => (
            "message".to_string(),
            format!("{:?} {}", direction, target),
        ),
    };

    AuditLogEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        agent_id: body.agent_id().to_string(),
        tool_name,
        operation,
        reason,
        risk_level,
        session_mode,
        disposition,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_log_entry_serialize_deserialize() {
        let entry = AuditLogEntry {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "agent-1".to_string(),
            tool_name: "file".to_string(),
            operation: "write /x".to_string(),
            reason: "approved by user".to_string(),
            risk_level: RiskLevel::High,
            session_mode: Some(SessionMode::Auto),
            disposition: AuditDisposition::Approved,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: AuditLogEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.agent_id, "agent-1");
        assert_eq!(parsed.tool_name, "file");
        assert_eq!(
            parsed.disposition,
            AuditDisposition::Approved
        );
        assert_eq!(parsed.risk_level, RiskLevel::High);
        assert_eq!(parsed.session_mode, Some(SessionMode::Auto));
    }

    #[test]
    fn test_audit_disposition_display() {
        assert_eq!(AuditDisposition::Approved.to_string(), "approved");
        assert_eq!(AuditDisposition::Rejected.to_string(), "rejected");
    }

    #[test]
    fn test_audit_disposition_json_roundtrip() {
        let approved = AuditDisposition::Approved;
        let json = serde_json::to_string(&approved).unwrap();
        assert_eq!(json, "\"approved\"");
        let parsed: AuditDisposition =
            serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AuditDisposition::Approved);

        let rejected = AuditDisposition::Rejected;
        let json = serde_json::to_string(&rejected).unwrap();
        assert_eq!(json, "\"rejected\"");
        let parsed: AuditDisposition =
            serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AuditDisposition::Rejected);
    }

    #[test]
    fn test_build_audit_log_file_op() {
        let body = PermissionRequestBody::FileOp {
            agent: "agent-1".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        };
        let log = build_audit_log(
            &body,
            AuditDisposition::Approved,
            "user approved".to_string(),
            RiskLevel::Low,
            None,
        );
        assert_eq!(log.agent_id, "agent-1");
        assert_eq!(log.tool_name, "file");
        assert_eq!(log.operation, "write /repo/src/main.rs");
        assert_eq!(log.reason, "user approved");
        assert_eq!(log.risk_level, RiskLevel::Low);
        assert_eq!(log.disposition, AuditDisposition::Approved);
        assert!(log.session_mode.is_none());
    }

    #[test]
    fn test_build_audit_log_command_exec() {
        let body = PermissionRequestBody::CommandExec {
            agent: "agent-2".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp".to_string()],
        };
        let log = build_audit_log(
            &body,
            AuditDisposition::Rejected,
            "command denied".to_string(),
            RiskLevel::High,
            Some(SessionMode::Auto),
        );
        assert_eq!(log.tool_name, "command");
        assert_eq!(log.operation, "rm -rf /tmp");
        assert_eq!(log.disposition, AuditDisposition::Rejected);
        assert_eq!(log.session_mode, Some(SessionMode::Auto));
    }

    #[test]
    fn test_file_audit_logger_writes_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path.clone()).unwrap();

        let entry = AuditLogEntry {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "agent-1".to_string(),
            tool_name: "file".to_string(),
            operation: "write /x".to_string(),
            reason: "approved".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Approved,
        };
        logger.log(&entry);

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: AuditLogEntry =
            serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.agent_id, "agent-1");
        assert_eq!(parsed.tool_name, "file");
        assert_eq!(
            parsed.disposition,
            AuditDisposition::Approved
        );
    }

    #[test]
    fn test_file_audit_logger_prepends_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path.clone()).unwrap();

        for i in 0..3 {
            let entry = AuditLogEntry {
                timestamp: format!(
                    "2026-01-01T00:00:{:02}Z",
                    i
                ),
                agent_id: format!("agent-{}", i),
                tool_name: "file".to_string(),
                operation: "write /x".to_string(),
                reason: "test".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Approved,
            };
            logger.log(&entry);
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> =
            content.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        // Newest first (reverse chronological order)
        for (i, line) in lines.iter().enumerate() {
            let parsed: AuditLogEntry =
                serde_json::from_str(line).unwrap();
            assert_eq!(
                parsed.agent_id,
                format!("agent-{}", 2 - i)
            );
        }
    }

    #[test]
    fn test_file_audit_logger_with_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger =
            FileAuditLogger::new_with_limit(path.clone(), Some(3))
                .unwrap();

        for i in 0..5 {
            let entry = AuditLogEntry {
                timestamp: format!(
                    "2026-01-01T00:00:{:02}Z",
                    i
                ),
                agent_id: format!("agent-{}", i),
                tool_name: "file".to_string(),
                operation: "write /x".to_string(),
                reason: "test".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Approved,
            };
            logger.log(&entry);
        }

        let count = FileAuditLogger::count_entries(&path);
        assert_eq!(count, 3);

        // Verify newest first
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> =
            content.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        for (i, line) in lines.iter().enumerate() {
            let parsed: AuditLogEntry =
                serde_json::from_str(line).unwrap();
            assert_eq!(
                parsed.agent_id,
                format!("agent-{}", 4 - i)
            );
        }
    }

    #[test]
    fn test_file_audit_logger_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path =
            dir.path().join("sub").join("dir").join("audit.log");
        let logger = FileAuditLogger::new(path.clone()).unwrap();
        let entry = AuditLogEntry {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "a".to_string(),
            tool_name: "f".to_string(),
            operation: "w x".to_string(),
            reason: "r".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Approved,
        };
        logger.log(&entry);
        assert!(path.exists());
    }

    #[test]
    fn test_file_audit_logger_debug_impl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();
        let debug_str = format!("{:?}", logger);
        assert!(debug_str.contains("FileAuditLogger"));
    }

    #[test]
    fn test_count_entries_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.log");
        std::fs::write(&path, "").unwrap();
        assert_eq!(FileAuditLogger::count_entries(&path), 0);
    }

    #[test]
    fn test_count_entries_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.log");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();
        assert_eq!(FileAuditLogger::count_entries(&path), 3);
    }

    #[test]
    fn test_count_entries_skips_blank_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.log");
        std::fs::write(&path, "line1\n\nline2\n  \nline3\n")
            .unwrap();
        assert_eq!(FileAuditLogger::count_entries(&path), 3);
    }

    #[test]
    fn test_new_with_limit_no_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new_with_limit(path, None).unwrap();
        assert_eq!(logger.max_entries(), None);
    }

    #[test]
    fn test_new_with_limit_with_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger =
            FileAuditLogger::new_with_limit(path, Some(5)).unwrap();
        assert_eq!(logger.max_entries(), Some(5));
    }
}
