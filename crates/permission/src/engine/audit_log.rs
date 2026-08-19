//! Audit logging for dangerous operations in Auto Mode.
//!
//! Records structured logs for both approved and rejected permission requests.
//! Follows the same JSON Lines format and truncation logic as `RejectionLog`.

use super::engine_risk::RiskLevel;
use super::engine_types::PermissionRequestBody;
use super::jsonl_writer::JsonlFileWriter;
use closeclaw_common::session_mode::SessionMode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
/// Delegates to [`JsonlFileWriter`] for shared write/truncation logic.
pub struct FileAuditLogger {
    inner: JsonlFileWriter,
}

impl FileAuditLogger {
    /// Create a new file logger that appends to the given path.
    /// Parent directories are created if they don't exist.
    pub fn new(path: PathBuf) -> std::io::Result<Self> {
        Self::new_with_limit(path, None)
    }

    /// Create a new file logger with a maximum entry limit.
    pub fn new_with_limit(path: PathBuf, max_entries: Option<usize>) -> std::io::Result<Self> {
        let inner = JsonlFileWriter::new_with_limit(path, max_entries)?;
        Ok(Self { inner })
    }

    /// Returns the path this logger writes to.
    pub fn path(&self) -> &PathBuf {
        self.inner.path()
    }

    /// Returns the configured maximum entry limit, if any.
    pub fn max_entries(&self) -> Option<usize> {
        self.inner.max_entries()
    }

    /// Count non-empty lines in the log file.
    pub fn count_entries(path: &PathBuf) -> usize {
        JsonlFileWriter::count_entries(path)
    }

    /// Read all audit log entries from the file.
    ///
    /// Returns entries in file order (newest first, matching write order).
    /// Returns an empty vec if the file does not exist or is empty.
    pub fn read_entries(&self) -> Vec<AuditLogEntry> {
        Self::read_entries_from_path(self.inner.path())
    }

    /// Read all audit log entries from the given path.
    pub fn read_entries_from_path(path: &PathBuf) -> Vec<AuditLogEntry> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// Query audit log entries with optional filters.
    ///
    /// All filter fields are optional; `None` means "no filter".
    /// Supports filtering by agent_id, disposition, and time range.
    pub fn query_entries(&self, filter: &AuditLogFilter) -> Vec<AuditLogEntry> {
        self.read_entries()
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect()
    }
}

/// Filter criteria for querying audit log entries.
#[derive(Debug, Clone, Default)]
pub struct AuditLogFilter {
    /// If set, only return entries for this agent.
    pub agent_id: Option<String>,
    /// If set, only return entries with this disposition.
    pub disposition: Option<AuditDisposition>,
    /// If set, only return entries with timestamp >= this value (ISO 8601).
    pub since: Option<String>,
    /// If set, only return entries with timestamp <= this value (ISO 8601).
    pub until: Option<String>,
}

impl AuditLogFilter {
    /// Check whether an entry matches this filter.
    pub fn matches(&self, entry: &AuditLogEntry) -> bool {
        if let Some(ref agent) = self.agent_id {
            if entry.agent_id != *agent {
                return false;
            }
        }
        if let Some(disp) = self.disposition {
            if entry.disposition != disp {
                return false;
            }
        }
        if let Some(ref since) = self.since {
            if entry.timestamp < *since {
                return false;
            }
        }
        if let Some(ref until) = self.until {
            if entry.timestamp > *until {
                return false;
            }
        }
        true
    }
}

impl AuditLogger for FileAuditLogger {
    fn log(&self, entry: &AuditLogEntry) {
        self.inner.write(entry);
    }
}

impl std::fmt::Debug for FileAuditLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileAuditLogger")
            .field("path", self.inner.path())
            .field("max_entries", &self.inner.max_entries())
            .finish()
    }
}

/// Build an [`AuditLogEntry`] from a request body and disposition
/// metadata.
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
        PermissionRequestBody::CommandExec { cmd, args, .. } => {
            ("command".to_string(), format!("{} {}", cmd, args.join(" ")))
        }
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
            direction, target, ..
        } => ("message".to_string(), format!("{:?} {}", direction, target)),
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
        assert_eq!(parsed.disposition, AuditDisposition::Approved);
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
        let parsed: AuditDisposition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AuditDisposition::Approved);

        let rejected = AuditDisposition::Rejected;
        let json = serde_json::to_string(&rejected).unwrap();
        assert_eq!(json, "\"rejected\"");
        let parsed: AuditDisposition = serde_json::from_str(&json).unwrap();
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
        let parsed: AuditLogEntry = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.agent_id, "agent-1");
        assert_eq!(parsed.tool_name, "file");
        assert_eq!(parsed.disposition, AuditDisposition::Approved);
    }

    #[test]
    fn test_file_audit_logger_prepends_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path.clone()).unwrap();

        for i in 0..3 {
            let entry = AuditLogEntry {
                timestamp: format!("2026-01-01T00:00:{:02}Z", i),
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
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        for (i, line) in lines.iter().enumerate() {
            let parsed: AuditLogEntry = serde_json::from_str(line).unwrap();
            assert_eq!(parsed.agent_id, format!("agent-{}", 2 - i));
        }
    }

    #[test]
    fn test_file_audit_logger_with_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new_with_limit(path.clone(), Some(3)).unwrap();

        for i in 0..5 {
            let entry = AuditLogEntry {
                timestamp: format!("2026-01-01T00:00:{:02}Z", i),
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

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        for (i, line) in lines.iter().enumerate() {
            let parsed: AuditLogEntry = serde_json::from_str(line).unwrap();
            assert_eq!(parsed.agent_id, format!("agent-{}", 4 - i));
        }
    }

    #[test]
    fn test_file_audit_logger_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("dir").join("audit.log");
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
        let logger = FileAuditLogger::new_with_limit(path, Some(5)).unwrap();
        assert_eq!(logger.max_entries(), Some(5));
    }

    // ------------------------------------------------------------------
    // read_entries / query_entries tests
    // ------------------------------------------------------------------

    fn make_entry(agent: &str, ts: &str, disp: AuditDisposition) -> AuditLogEntry {
        AuditLogEntry {
            timestamp: ts.to_string(),
            agent_id: agent.to_string(),
            tool_name: "file".to_string(),
            operation: "write /x".to_string(),
            reason: "test".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: disp,
        }
    }

    #[test]
    fn test_read_entries_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        std::fs::write(&path, "").unwrap();
        let logger = FileAuditLogger::new(path).unwrap();
        let entries = logger.read_entries();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_read_entries_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does_not_exist.log");
        let logger = FileAuditLogger::new(path).unwrap();
        let entries = logger.read_entries();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_read_entries_after_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        logger.log(&make_entry(
            "a1",
            "2026-01-01T00:00:00Z",
            AuditDisposition::Approved,
        ));
        logger.log(&make_entry(
            "a2",
            "2026-01-01T00:01:00Z",
            AuditDisposition::Rejected,
        ));

        let entries = logger.read_entries();
        assert_eq!(entries.len(), 2);
        // newest first
        assert_eq!(entries[0].agent_id, "a2");
        assert_eq!(entries[1].agent_id, "a1");
    }

    #[test]
    fn test_read_entries_from_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path.clone()).unwrap();
        logger.log(&make_entry(
            "x",
            "2026-02-01T00:00:00Z",
            AuditDisposition::Approved,
        ));

        let entries = FileAuditLogger::read_entries_from_path(&path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "x");
    }

    #[test]
    fn test_query_entries_by_agent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        logger.log(&make_entry(
            "a1",
            "2026-01-01T00:00:00Z",
            AuditDisposition::Approved,
        ));
        logger.log(&make_entry(
            "a2",
            "2026-01-01T00:01:00Z",
            AuditDisposition::Rejected,
        ));
        logger.log(&make_entry(
            "a1",
            "2026-01-01T00:02:00Z",
            AuditDisposition::Approved,
        ));

        let filter = AuditLogFilter {
            agent_id: Some("a1".to_string()),
            ..Default::default()
        };
        let results = logger.query_entries(&filter);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|e| e.agent_id == "a1"));
    }

    #[test]
    fn test_query_entries_by_disposition() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        logger.log(&make_entry(
            "a1",
            "2026-01-01T00:00:00Z",
            AuditDisposition::Approved,
        ));
        logger.log(&make_entry(
            "a2",
            "2026-01-01T00:01:00Z",
            AuditDisposition::Rejected,
        ));
        logger.log(&make_entry(
            "a3",
            "2026-01-01T00:02:00Z",
            AuditDisposition::Approved,
        ));

        let filter = AuditLogFilter {
            disposition: Some(AuditDisposition::Rejected),
            ..Default::default()
        };
        let results = logger.query_entries(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].agent_id, "a2");
    }

    #[test]
    fn test_query_entries_by_time_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        logger.log(&make_entry(
            "a1",
            "2026-01-01T00:00:00Z",
            AuditDisposition::Approved,
        ));
        logger.log(&make_entry(
            "a2",
            "2026-01-02T00:00:00Z",
            AuditDisposition::Rejected,
        ));
        logger.log(&make_entry(
            "a3",
            "2026-01-03T00:00:00Z",
            AuditDisposition::Approved,
        ));

        let filter = AuditLogFilter {
            since: Some("2026-01-02T00:00:00Z".to_string()),
            until: Some("2026-01-03T00:00:00Z".to_string()),
            ..Default::default()
        };
        let results = logger.query_entries(&filter);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].agent_id, "a3");
        assert_eq!(results[1].agent_id, "a2");
    }

    #[test]
    fn test_query_entries_combined_filters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        logger.log(&make_entry(
            "a1",
            "2026-01-01T10:00:00Z",
            AuditDisposition::Approved,
        ));
        logger.log(&make_entry(
            "a1",
            "2026-01-01T12:00:00Z",
            AuditDisposition::Rejected,
        ));
        logger.log(&make_entry(
            "a2",
            "2026-01-01T14:00:00Z",
            AuditDisposition::Approved,
        ));

        let filter = AuditLogFilter {
            agent_id: Some("a1".to_string()),
            disposition: Some(AuditDisposition::Rejected),
            ..Default::default()
        };
        let results = logger.query_entries(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].agent_id, "a1");
        assert_eq!(results[0].disposition, AuditDisposition::Rejected);
    }

    #[test]
    fn test_query_entries_empty_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        let filter = AuditLogFilter {
            agent_id: Some("a1".to_string()),
            ..Default::default()
        };
        let results = logger.query_entries(&filter);
        assert!(results.is_empty());
    }

    #[test]
    fn test_audit_log_filter_default() {
        let filter = AuditLogFilter::default();
        assert!(filter.agent_id.is_none());
        assert!(filter.disposition.is_none());
        assert!(filter.since.is_none());
        assert!(filter.until.is_none());
    }

    #[test]
    fn test_audit_log_filter_matches_all_none() {
        let filter = AuditLogFilter::default();
        let entry = make_entry("a", "2026-01-01T00:00:00Z", AuditDisposition::Approved);
        assert!(filter.matches(&entry));
    }

    // ------------------------------------------------------------------
    // Supplementary: mixed valid/invalid JSON lines
    // ------------------------------------------------------------------

    /// read_entries should skip malformed JSON lines gracefully.
    #[test]
    fn test_read_entries_skips_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path.clone()).unwrap();

        logger.log(&make_entry(
            "a1",
            "2026-01-01T00:00:00Z",
            AuditDisposition::Approved,
        ));

        // Append a malformed line after the valid entry
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push_str("not valid json\n");
        std::fs::write(&path, &content).unwrap();

        logger.log(&make_entry(
            "a2",
            "2026-01-01T00:01:00Z",
            AuditDisposition::Rejected,
        ));

        let entries = logger.read_entries();
        // Should have 2 valid entries, skipping the malformed line
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].agent_id, "a2");
        assert_eq!(entries[1].agent_id, "a1");
    }

    /// read_entries should handle blank lines between entries.
    #[test]
    fn test_read_entries_skips_blank_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        // Write entries in file order (newest first, matching writer prepend behavior)
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"t2\",\"agent_id\":\"a2\",\"tool_name\":\"f\",\"operation\":\"w x\",\"reason\":\"r\",\"risk_level\":\"low\",\"disposition\":\"rejected\"}\n",
                "\n",
                "\n",
                "{\"timestamp\":\"t1\",\"agent_id\":\"a1\",\"tool_name\":\"f\",\"operation\":\"w x\",\"reason\":\"r\",\"risk_level\":\"low\",\"disposition\":\"approved\"}\n",
            ),
        )
        .unwrap();
        let logger = FileAuditLogger::new(path).unwrap();
        let entries = logger.read_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].agent_id, "a2");
        assert_eq!(entries[1].agent_id, "a1");
    }

    /// Multiple write-then-read cycles accumulate entries correctly.
    #[test]
    fn test_read_entries_multiple_write_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        // First cycle: write 2 entries, read
        logger.log(&make_entry(
            "cycle1",
            "2026-01-01T00:00:00Z",
            AuditDisposition::Approved,
        ));
        logger.log(&make_entry(
            "cycle1",
            "2026-01-01T00:01:00Z",
            AuditDisposition::Rejected,
        ));
        assert_eq!(logger.read_entries().len(), 2);

        // Second cycle: write 1 more, read
        logger.log(&make_entry(
            "cycle2",
            "2026-01-01T00:02:00Z",
            AuditDisposition::Approved,
        ));
        let entries = logger.read_entries();
        assert_eq!(entries.len(), 3);
        // Newest first: cycle2, cycle1 (rejected), cycle1 (approved)
        assert_eq!(entries[0].agent_id, "cycle2");
        assert_eq!(entries[0].disposition, AuditDisposition::Approved);
    }

    /// Query with all filter fields set returns only matching entries.
    #[test]
    fn test_query_entries_all_filters_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        logger.log(&make_entry(
            "a1",
            "2026-01-01T00:00:00Z",
            AuditDisposition::Approved,
        ));

        let filter = AuditLogFilter {
            agent_id: Some("nonexistent".to_string()),
            disposition: Some(AuditDisposition::Rejected),
            since: Some("2026-06-01T00:00:00Z".to_string()),
            until: Some("2026-12-31T23:59:59Z".to_string()),
        };
        let results = logger.query_entries(&filter);
        assert!(results.is_empty());
    }

    /// Single entry write then read.
    #[test]
    fn test_read_entries_single_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let logger = FileAuditLogger::new(path).unwrap();

        logger.log(&make_entry(
            "solo",
            "2026-03-15T12:00:00Z",
            AuditDisposition::Rejected,
        ));

        let entries = logger.read_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "solo");
        assert_eq!(entries[0].disposition, AuditDisposition::Rejected);
        assert_eq!(entries[0].timestamp, "2026-03-15T12:00:00Z");
    }
}
