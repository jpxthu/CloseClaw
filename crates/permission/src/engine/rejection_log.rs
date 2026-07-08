//! Rejection logging for PermissionEngine.
//!
//! Records structured logs when permission requests are denied.

use super::engine_risk::RiskLevel;
use super::engine_types::PermissionRequestBody;
use closeclaw_common::session_mode::SessionMode;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// Structured rejection log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionLog {
    /// Timestamp of the rejection (ISO 8601).
    pub timestamp: String,
    /// Agent ID that was denied.
    pub agent_id: String,
    /// Tool/request type name.
    pub tool_name: String,
    /// Operation description (e.g. "write", "read", command text).
    pub operation: String,
    /// Human-readable reason for denial.
    pub reason: String,
    /// Risk level of the denied operation.
    pub risk_level: RiskLevel,
    /// Session mode at the time of denial.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_mode: Option<SessionMode>,
}

/// Trait for recording rejection log entries.
pub trait RejectionLogger: Send + Sync {
    /// Log a rejection entry.
    fn log(&self, entry: &RejectionLog);
}

/// File-based rejection logger using JSON Lines format.
pub struct FileRejectionLogger {
    path: PathBuf,
    writer: Mutex<Box<dyn Write + Send>>,
}

impl FileRejectionLogger {
    /// Create a new file logger that appends to the given path.
    /// Parent directories are created if they don't exist.
    pub fn new(path: PathBuf) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: Mutex::new(Box::new(file)),
        })
    }

    /// Returns the path this logger writes to.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl RejectionLogger for FileRejectionLogger {
    fn log(&self, entry: &RejectionLog) {
        if let Ok(mut writer) = self.writer.lock() {
            let mut line = serde_json::to_vec(entry).unwrap_or_default();
            line.push(b'\n');
            let _ = writer.write_all(&line);
        }
    }
}

impl std::fmt::Debug for FileRejectionLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRejectionLogger")
            .field("path", &self.path)
            .finish()
    }
}

/// Build a [`RejectionLog`] from a request body and denial metadata.
pub fn build_rejection_log(
    body: &PermissionRequestBody,
    reason: String,
    risk_level: RiskLevel,
    session_mode: Option<SessionMode>,
) -> RejectionLog {
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
    };

    RejectionLog {
        timestamp: chrono::Utc::now().to_rfc3339(),
        agent_id: body.agent_id().to_string(),
        tool_name,
        operation,
        reason,
        risk_level,
        session_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory logger for testing.
    struct MockRejectionLogger {
        entries: Mutex<Vec<RejectionLog>>,
    }

    impl MockRejectionLogger {
        fn new() -> Self {
            Self {
                entries: Mutex::new(Vec::new()),
            }
        }

        fn entries(&self) -> Vec<RejectionLog> {
            self.entries.lock().unwrap().clone()
        }
    }

    impl RejectionLogger for MockRejectionLogger {
        fn log(&self, entry: &RejectionLog) {
            self.entries.lock().unwrap().push(entry.clone());
        }
    }

    #[test]
    fn test_build_rejection_log_file_op() {
        let body = PermissionRequestBody::FileOp {
            agent: "agent-1".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        };
        let log = build_rejection_log(&body, "denied".to_string(), RiskLevel::Low, None);
        assert_eq!(log.agent_id, "agent-1");
        assert_eq!(log.tool_name, "file");
        assert_eq!(log.operation, "write /repo/src/main.rs");
        assert_eq!(log.reason, "denied");
        assert_eq!(log.risk_level, RiskLevel::Low);
        assert!(log.session_mode.is_none());
    }

    #[test]
    fn test_build_rejection_log_command_exec() {
        let body = PermissionRequestBody::CommandExec {
            agent: "agent-2".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp".to_string()],
        };
        let log = build_rejection_log(
            &body,
            "command denied".to_string(),
            RiskLevel::High,
            Some(SessionMode::Plan),
        );
        assert_eq!(log.tool_name, "command");
        assert_eq!(log.operation, "rm -rf /tmp");
        assert_eq!(log.session_mode, Some(SessionMode::Plan));
    }

    #[test]
    fn test_mock_logger_records_entries() {
        let logger = MockRejectionLogger::new();
        let entry = RejectionLog {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "a".to_string(),
            tool_name: "file".to_string(),
            operation: "write x".to_string(),
            reason: "test".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
        };
        logger.log(&entry);
        let entries = logger.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "a");
    }

    #[test]
    fn test_file_logger_writes_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rejections.log");
        let logger = FileRejectionLogger::new(path.clone()).unwrap();

        let entry = RejectionLog {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "agent-1".to_string(),
            tool_name: "file".to_string(),
            operation: "write /x".to_string(),
            reason: "denied".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
        };
        logger.log(&entry);

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: RejectionLog = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.agent_id, "agent-1");
        assert_eq!(parsed.tool_name, "file");
    }

    #[test]
    fn test_file_logger_appends_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rejections.log");
        let logger = FileRejectionLogger::new(path.clone()).unwrap();

        for i in 0..3 {
            let entry = RejectionLog {
                timestamp: format!("2026-01-01T00:00:{:02}Z", i),
                agent_id: format!("agent-{}", i),
                tool_name: "file".to_string(),
                operation: "write /x".to_string(),
                reason: "denied".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
            };
            logger.log(&entry);
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        for (i, line) in lines.iter().enumerate() {
            let parsed: RejectionLog = serde_json::from_str(line).unwrap();
            assert_eq!(parsed.agent_id, format!("agent-{}", i));
        }
    }

    #[test]
    fn test_file_logger_debug_impl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rejections.log");
        let logger = FileRejectionLogger::new(path.clone()).unwrap();
        let debug_str = format!("{:?}", logger);
        assert!(debug_str.contains("FileRejectionLogger"));
    }

    #[test]
    fn test_file_logger_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("dir").join("rejections.log");
        let logger = FileRejectionLogger::new(path.clone()).unwrap();
        let entry = RejectionLog {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "a".to_string(),
            tool_name: "f".to_string(),
            operation: "w x".to_string(),
            reason: "r".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
        };
        logger.log(&entry);
        assert!(path.exists());
    }
}
