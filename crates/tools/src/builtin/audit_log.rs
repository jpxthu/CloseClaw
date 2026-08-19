//! Built-in meta tool — AuditLogTool.
//!
//! Allows the LLM to query the audit log for permission decisions
//! in Auto Mode.

use crate::{Tool, ToolCallError, ToolFlags, ToolResult};

use async_trait::async_trait;
use closeclaw_common::tool_trait::ToolContext;
use closeclaw_permission::engine::audit_log::{AuditDisposition, AuditLogFilter, FileAuditLogger};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// AuditLogTool
// ---------------------------------------------------------------------------

/// Tool that reads and filters audit log entries.
///
/// In Auto Mode, dangerous operations that trigger approval produce
/// audit log entries. This tool lets the LLM inspect those entries.
pub struct AuditLogTool {
    logger: Arc<RwLock<FileAuditLogger>>,
}

impl AuditLogTool {
    /// Create a new `AuditLogTool` with the given audit log path.
    pub fn new(audit_log_path: PathBuf) -> Self {
        let logger =
            FileAuditLogger::new(audit_log_path).expect("failed to create audit log reader");
        Self {
            logger: Arc::new(RwLock::new(logger)),
        }
    }

    /// Create with an existing logger (for testing).
    pub fn with_logger(logger: Arc<RwLock<FileAuditLogger>>) -> Self {
        Self { logger }
    }
}

#[async_trait]
impl Tool for AuditLogTool {
    fn name(&self) -> &str {
        "AuditLog"
    }

    fn group(&self) -> &str {
        "meta"
    }

    fn summary(&self) -> String {
        "Query audit log for permission decisions".to_string()
    }

    fn detail(&self) -> String {
        "Query the audit log to review permission decisions made in Auto Mode.\
         \n\nReturns audit log entries recording approved and rejected operations.\
         Supports filtering by agent_id, disposition, and time range.\
         \n\nThe audit log tracks all dangerous operations that triggered approval\
         flows in Auto Mode, providing transparency into what was allowed or denied.\
         \n\nFilter parameters are all optional. If none are provided, all entries\
         are returned."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Filter by agent ID"
                },
                "disposition": {
                    "type": "string",
                    "enum": ["approved", "rejected"],
                    "description": "Filter by disposition (approved or rejected)"
                },
                "since": {
                    "type": "string",
                    "description": "Only return entries with timestamp >= this ISO 8601 value"
                },
                "until": {
                    "type": "string",
                    "description": "Only return entries with timestamp <= this ISO 8601 value"
                }
            },
            "required": []
        })
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let filter = parse_filter(&args);
        let logger = self.logger.read().await;
        let entries = logger.query_entries(&filter);

        let data = serde_json::json!({
            "total": entries.len(),
            "entries": entries,
        });

        Ok(ToolResult {
            data,
            new_messages: vec![],
            context_modifier: None,
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            is_destructive: false,
            is_expensive: false,
            is_deferred_by_default: false,
        }
    }
}

/// Parse filter parameters from tool call arguments.
fn parse_filter(args: &Value) -> AuditLogFilter {
    let agent_id = args
        .get("agent_id")
        .and_then(Value::as_str)
        .map(String::from);

    let disposition = args
        .get("disposition")
        .and_then(Value::as_str)
        .and_then(|s| match s {
            "approved" => Some(AuditDisposition::Approved),
            "rejected" => Some(AuditDisposition::Rejected),
            _ => None,
        });

    let since = args.get("since").and_then(Value::as_str).map(String::from);

    let until = args.get("until").and_then(Value::as_str).map(String::from);

    AuditLogFilter {
        agent_id,
        disposition,
        since,
        until,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_permission::engine::audit_log::AuditLogEntry;
    use closeclaw_permission::engine::engine_risk::RiskLevel;
    use closeclaw_permission::AuditLogger;

    fn make_test_logger(dir: &std::path::Path) -> Arc<RwLock<FileAuditLogger>> {
        let path = dir.join("audit.log");
        Arc::new(RwLock::new(FileAuditLogger::new(path).unwrap()))
    }

    fn write_test_entry(logger: &FileAuditLogger, agent: &str, ts: &str) {
        logger.log(&AuditLogEntry {
            timestamp: ts.to_string(),
            agent_id: agent.to_string(),
            tool_name: "file".to_string(),
            operation: "write /x".to_string(),
            reason: "test".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Approved,
        });
    }

    #[test]
    fn test_audit_log_tool_name_group() {
        let dir = tempfile::tempdir().unwrap();
        let tool = AuditLogTool::new(dir.path().join("audit.log"));
        assert_eq!(tool.name(), "AuditLog");
        assert_eq!(tool.group(), "meta");
    }

    #[test]
    fn test_audit_log_tool_summary_len() {
        let dir = tempfile::tempdir().unwrap();
        let tool = AuditLogTool::new(dir.path().join("audit.log"));
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_audit_log_tool_flags() {
        let dir = tempfile::tempdir().unwrap();
        let tool = AuditLogTool::new(dir.path().join("audit.log"));
        let flags = tool.flags();
        assert!(flags.is_read_only);
        assert!(flags.is_concurrency_safe);
        assert!(!flags.is_deferred_by_default);
        assert!(!flags.is_destructive);
    }

    #[test]
    fn test_audit_log_tool_input_schema_no_required() {
        let dir = tempfile::tempdir().unwrap();
        let tool = AuditLogTool::new(dir.path().join("audit.log"));
        let schema = tool.input_schema();
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn test_audit_log_tool_input_schema_has_filters() {
        let dir = tempfile::tempdir().unwrap();
        let tool = AuditLogTool::new(dir.path().join("audit.log"));
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("agent_id"));
        assert!(props.contains_key("disposition"));
        assert!(props.contains_key("since"));
        assert!(props.contains_key("until"));
    }

    #[test]
    fn test_audit_log_tool_detail_mentions_audit() {
        let dir = tempfile::tempdir().unwrap();
        let tool = AuditLogTool::new(dir.path().join("audit.log"));
        let detail = tool.detail();
        assert!(detail.contains("audit"));
        assert!(detail.contains("Auto Mode"));
    }

    #[test]
    fn test_parse_filter_empty() {
        let args = serde_json::json!({});
        let filter = parse_filter(&args);
        assert!(filter.agent_id.is_none());
        assert!(filter.disposition.is_none());
        assert!(filter.since.is_none());
        assert!(filter.until.is_none());
    }

    #[test]
    fn test_parse_filter_all_fields() {
        let args = serde_json::json!({
            "agent_id": "a1",
            "disposition": "rejected",
            "since": "2026-01-01T00:00:00Z",
            "until": "2026-12-31T23:59:59Z"
        });
        let filter = parse_filter(&args);
        assert_eq!(filter.agent_id.as_deref(), Some("a1"));
        assert_eq!(filter.disposition, Some(AuditDisposition::Rejected));
        assert_eq!(filter.since.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(filter.until.as_deref(), Some("2026-12-31T23:59:59Z"));
    }

    #[test]
    fn test_parse_filter_invalid_disposition() {
        let args = serde_json::json!({"disposition": "invalid"});
        let filter = parse_filter(&args);
        assert!(filter.disposition.is_none());
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_empty() {
        let dir = tempfile::tempdir().unwrap();
        let logger = make_test_logger(dir.path());
        let tool = AuditLogTool::with_logger(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let result = tool.call(serde_json::json!({}), &ctx).await.unwrap();
        assert_eq!(result.data["total"], 0);
        assert!(result.data["entries"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_returns_entries() {
        let dir = tempfile::tempdir().unwrap();
        let logger = make_test_logger(dir.path());
        {
            let l = logger.read().await;
            write_test_entry(&l, "a1", "2026-01-01T00:00:00Z");
            write_test_entry(&l, "a2", "2026-01-01T00:01:00Z");
        }
        let tool = AuditLogTool::with_logger(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let result = tool.call(serde_json::json!({}), &ctx).await.unwrap();
        assert_eq!(result.data["total"], 2);
        let entries = result.data["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_with_filter() {
        let dir = tempfile::tempdir().unwrap();
        let logger = make_test_logger(dir.path());
        {
            let l = logger.read().await;
            write_test_entry(&l, "a1", "2026-01-01T00:00:00Z");
            write_test_entry(&l, "a2", "2026-01-01T00:01:00Z");
        }
        let tool = AuditLogTool::with_logger(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let result = tool
            .call(serde_json::json!({"agent_id": "a1"}), &ctx)
            .await
            .unwrap();
        assert_eq!(result.data["total"], 1);
        assert_eq!(result.data["entries"][0]["agent_id"], "a1");
    }

    // ------------------------------------------------------------------
    // Supplementary: disposition filter + time range filter via tool call
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_audit_log_tool_call_with_disposition_filter() {
        let dir = tempfile::tempdir().unwrap();
        let logger = make_test_logger(dir.path());
        {
            let l = logger.read().await;
            l.log(&AuditLogEntry {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                agent_id: "a1".to_string(),
                tool_name: "file".to_string(),
                operation: "write /x".to_string(),
                reason: "approved".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Approved,
            });
            l.log(&AuditLogEntry {
                timestamp: "2026-01-01T00:01:00Z".to_string(),
                agent_id: "a2".to_string(),
                tool_name: "command".to_string(),
                operation: "rm /tmp".to_string(),
                reason: "denied".to_string(),
                risk_level: RiskLevel::High,
                session_mode: None,
                disposition: AuditDisposition::Rejected,
            });
        }
        let tool = AuditLogTool::with_logger(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let result = tool
            .call(serde_json::json!({"disposition": "rejected"}), &ctx)
            .await
            .unwrap();
        assert_eq!(result.data["total"], 1);
        assert_eq!(result.data["entries"][0]["agent_id"], "a2");
        assert_eq!(result.data["entries"][0]["disposition"], "rejected");
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_with_time_range_filter() {
        let dir = tempfile::tempdir().unwrap();
        let logger = make_test_logger(dir.path());
        {
            let l = logger.read().await;
            l.log(&AuditLogEntry {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                agent_id: "early".to_string(),
                tool_name: "file".to_string(),
                operation: "w".to_string(),
                reason: "r".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Approved,
            });
            l.log(&AuditLogEntry {
                timestamp: "2026-06-15T12:00:00Z".to_string(),
                agent_id: "mid".to_string(),
                tool_name: "file".to_string(),
                operation: "w".to_string(),
                reason: "r".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Approved,
            });
            l.log(&AuditLogEntry {
                timestamp: "2026-12-31T23:59:59Z".to_string(),
                agent_id: "late".to_string(),
                tool_name: "file".to_string(),
                operation: "w".to_string(),
                reason: "r".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Approved,
            });
        }
        let tool = AuditLogTool::with_logger(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let result = tool
            .call(
                serde_json::json!({
                    "since": "2026-03-01T00:00:00Z",
                    "until": "2026-09-01T00:00:00Z"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.data["total"], 1);
        assert_eq!(result.data["entries"][0]["agent_id"], "mid");
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_combined_filters() {
        let dir = tempfile::tempdir().unwrap();
        let logger = make_test_logger(dir.path());
        {
            let l = logger.read().await;
            l.log(&AuditLogEntry {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                agent_id: "a1".to_string(),
                tool_name: "file".to_string(),
                operation: "w".to_string(),
                reason: "r".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Approved,
            });
            l.log(&AuditLogEntry {
                timestamp: "2026-06-01T00:00:00Z".to_string(),
                agent_id: "a1".to_string(),
                tool_name: "file".to_string(),
                operation: "w".to_string(),
                reason: "r".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Rejected,
            });
            l.log(&AuditLogEntry {
                timestamp: "2026-06-01T00:00:00Z".to_string(),
                agent_id: "a2".to_string(),
                tool_name: "file".to_string(),
                operation: "w".to_string(),
                reason: "r".to_string(),
                risk_level: RiskLevel::Low,
                session_mode: None,
                disposition: AuditDisposition::Rejected,
            });
        }
        let tool = AuditLogTool::with_logger(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let result = tool
            .call(
                serde_json::json!({
                    "agent_id": "a1",
                    "disposition": "rejected"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.data["total"], 1);
        assert_eq!(result.data["entries"][0]["agent_id"], "a1");
        assert_eq!(result.data["entries"][0]["disposition"], "rejected");
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_no_entries_matching() {
        let dir = tempfile::tempdir().unwrap();
        let logger = make_test_logger(dir.path());
        {
            let l = logger.read().await;
            write_test_entry(&l, "a1", "2026-01-01T00:00:00Z");
        }
        let tool = AuditLogTool::with_logger(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let result = tool
            .call(serde_json::json!({"agent_id": "nonexistent"}), &ctx)
            .await
            .unwrap();
        assert_eq!(result.data["total"], 0);
        assert!(result.data["entries"].as_array().unwrap().is_empty());
    }
}
