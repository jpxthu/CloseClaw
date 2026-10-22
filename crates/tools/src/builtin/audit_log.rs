//! Built-in meta tool — AuditLogTool.
//!
//! Allows the LLM to query the audit log for permission decisions
//! in Auto Mode.

use crate::{Tool, ToolCallError, ToolFlags, ToolResult};

use async_trait::async_trait;
use closeclaw_common::audit_log::{AuditDisposition, AuditLogFilter, AuditLogger};
use closeclaw_common::tool_trait::ToolContext;
use serde_json::Value;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// AuditLogTool
// ---------------------------------------------------------------------------

/// Tool that reads and filters audit log entries.
///
/// In Auto Mode, dangerous operations that trigger approval produce
/// audit log entries. This tool lets the LLM inspect those entries.
pub struct AuditLogTool {
    logger: Arc<dyn AuditLogger>,
}

impl AuditLogTool {
    /// Create a new `AuditLogTool` reading through the given logger.
    pub fn new(logger: Arc<dyn AuditLogger>) -> Self {
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
                    "description": "Only return entries with timestamp \
        >= this value (RFC 3339, e.g. 2026-01-01T00:00:00Z)"
                },
                "until": {
                    "type": "string",
                    "description": "Only return entries with timestamp \
        <= this value (RFC 3339, e.g. 2026-01-01T00:00:00Z)"
                }
            },
            "required": []
        })
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let filter = parse_filter(&args)?;
        let entries = self.logger.query_entries(&filter);

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
///
/// Returns `ToolCallError::InvalidArgs` when a string `since`/`until`
/// is not valid RFC 3339 (message names the field, quotes the value and
/// states the expected format); a missing or non-string value stays
/// `None` (no constraint).
fn parse_filter(args: &Value) -> Result<AuditLogFilter, ToolCallError> {
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

    let parse_time_bound = |key: &str| -> Result<Option<String>, ToolCallError> {
        match args.get(key).and_then(Value::as_str) {
            Some(raw) => {
                chrono::DateTime::parse_from_rfc3339(raw).map_err(|e| {
                    ToolCallError::InvalidArgs(format!(
                        "invalid {key} filter: {raw:?} is not a valid ISO 8601 / \
                         RFC 3339 timestamp (e.g. 2026-01-01T00:00:00Z or \
                         2026-01-01T08:00:00+08:00): {e}"
                    ))
                })?;
                Ok(Some(raw.to_string()))
            }
            None => Ok(None),
        }
    };

    let since = parse_time_bound("since")?;
    let until = parse_time_bound("until")?;

    Ok(AuditLogFilter {
        agent_id,
        disposition,
        since,
        until,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::audit_log::AuditLogEntry;
    use closeclaw_common::permission_types::RiskLevel;
    use std::sync::Mutex;

    /// In-memory `AuditLogger` fake mirroring `FileAuditLogger` semantics:
    /// filtering via `AuditLogFilter::matches`, newest first.
    struct MemAuditLogger {
        entries: Mutex<Vec<AuditLogEntry>>,
    }

    impl MemAuditLogger {
        fn new() -> Self {
            Self {
                entries: Mutex::new(Vec::new()),
            }
        }
    }

    impl AuditLogger for MemAuditLogger {
        fn log(&self, entry: &AuditLogEntry) {
            self.entries.lock().unwrap().push(entry.clone());
        }

        fn query_entries(&self, filter: &AuditLogFilter) -> Vec<AuditLogEntry> {
            let entries = self.entries.lock().unwrap();
            let mut matched: Vec<AuditLogEntry> = entries
                .iter()
                .filter(|e| filter.matches(e))
                .cloned()
                .collect();
            matched.reverse();
            matched
        }
    }

    fn make_test_logger() -> Arc<dyn AuditLogger> {
        Arc::new(MemAuditLogger::new())
    }

    fn write_test_entry(logger: &dyn AuditLogger, agent: &str, ts: &str) {
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
        let tool = AuditLogTool::new(make_test_logger());
        assert_eq!(tool.name(), "AuditLog");
        assert_eq!(tool.group(), "meta");
    }

    #[test]
    fn test_audit_log_tool_summary_len() {
        let tool = AuditLogTool::new(make_test_logger());
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_audit_log_tool_flags() {
        let tool = AuditLogTool::new(make_test_logger());
        let flags = tool.flags();
        assert!(flags.is_read_only);
        assert!(flags.is_concurrency_safe);
        assert!(!flags.is_deferred_by_default);
        assert!(!flags.is_destructive);
    }

    #[test]
    fn test_audit_log_tool_input_schema_no_required() {
        let tool = AuditLogTool::new(make_test_logger());
        let schema = tool.input_schema();
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn test_audit_log_tool_input_schema_has_filters() {
        let tool = AuditLogTool::new(make_test_logger());
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("agent_id"));
        assert!(props.contains_key("disposition"));
        assert!(props.contains_key("since"));
        assert!(props.contains_key("until"));
    }

    /// Schema description of `since`/`until` must match the actual
    /// `parse_filter` validation contract: RFC 3339 (not loose ISO 8601),
    /// with the correct comparison direction and a copyable example, so
    /// that LLM-produced arguments do not trigger an avoidable error round.
    #[test]
    fn test_audit_log_tool_input_schema_time_bounds_rfc3339() {
        let tool = AuditLogTool::new(make_test_logger());
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();

        for (key, op) in [("since", ">="), ("until", "<=")] {
            let desc = props[key]["description"].as_str().unwrap();
            assert!(
                desc.contains("RFC 3339") && desc.contains(op),
                "{key} description should state RFC 3339 and {op}: {desc}"
            );
            assert!(
                desc.contains("2026-01-01T00:00:00Z"),
                "{key} description should include an RFC 3339 example: {desc}"
            );
        }
    }

    #[test]
    fn test_audit_log_tool_detail_mentions_audit() {
        let tool = AuditLogTool::new(make_test_logger());
        let detail = tool.detail();
        assert!(detail.contains("audit"));
        assert!(detail.contains("Auto Mode"));
    }

    #[test]
    fn test_parse_filter_empty() {
        let args = serde_json::json!({});
        let filter = parse_filter(&args).unwrap();
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
        let filter = parse_filter(&args).unwrap();
        assert_eq!(filter.agent_id.as_deref(), Some("a1"));
        assert_eq!(filter.disposition, Some(AuditDisposition::Rejected));
        assert_eq!(filter.since.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(filter.until.as_deref(), Some("2026-12-31T23:59:59Z"));
    }

    #[test]
    fn test_parse_filter_invalid_disposition() {
        let args = serde_json::json!({"disposition": "invalid"});
        let filter = parse_filter(&args).unwrap();
        assert!(filter.disposition.is_none());
    }

    #[test]
    fn test_parse_filter_invalid_since_rejected_with_format_hint() {
        let args = serde_json::json!({"since": "not-a-time"});
        let err = parse_filter(&args).unwrap_err();
        let ToolCallError::InvalidArgs(msg) = err else {
            panic!("expected InvalidArgs, got {err:?}");
        };
        assert!(
            msg.contains("since"),
            "message should name the field: {msg}"
        );
        assert!(
            msg.contains("RFC 3339"),
            "message should contain format hint: {msg}"
        );
        assert!(
            msg.contains("not-a-time"),
            "message should quote the value: {msg}"
        );
    }

    #[test]
    fn test_parse_filter_invalid_until_rejected_with_format_hint() {
        let args = serde_json::json!({"until": "2026/06/15 12:00:00"});
        let err = parse_filter(&args).unwrap_err();
        let ToolCallError::InvalidArgs(msg) = err else {
            panic!("expected InvalidArgs, got {err:?}");
        };
        assert!(
            msg.contains("until"),
            "message should name the field: {msg}"
        );
        assert!(
            msg.contains("RFC 3339"),
            "message should contain format hint: {msg}"
        );
        assert!(
            msg.contains("2026/06/15 12:00:00"),
            "message should quote the value: {msg}"
        );
    }

    #[test]
    fn test_parse_filter_valid_mixed_iso8601_forms_preserved() {
        let args = serde_json::json!({
            "since": "2026-01-01T00:00:00.123456789Z",
            "until": "2026-12-31T23:59:59+08:00"
        });
        let filter = parse_filter(&args).unwrap();
        assert_eq!(
            filter.since.as_deref(),
            Some("2026-01-01T00:00:00.123456789Z")
        );
        assert_eq!(filter.until.as_deref(), Some("2026-12-31T23:59:59+08:00"));
    }

    #[test]
    fn test_parse_filter_missing_or_non_string_time_is_none() {
        let args = serde_json::json!({"since": 12345, "until": true});
        let filter = parse_filter(&args).unwrap();
        assert!(filter.since.is_none());
        assert!(filter.until.is_none());
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_empty() {
        let logger = make_test_logger();
        let tool = AuditLogTool::new(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let result = tool.call(serde_json::json!({}), &ctx).await.unwrap();
        assert_eq!(result.data["total"], 0);
        assert!(result.data["entries"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_returns_entries() {
        let logger = make_test_logger();
        write_test_entry(&*logger, "a1", "2026-01-01T00:00:00Z");
        write_test_entry(&*logger, "a2", "2026-01-01T00:01:00Z");
        let tool = AuditLogTool::new(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let result = tool.call(serde_json::json!({}), &ctx).await.unwrap();
        assert_eq!(result.data["total"], 2);
        let entries = result.data["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_with_filter() {
        let logger = make_test_logger();
        write_test_entry(&*logger, "a1", "2026-01-01T00:00:00Z");
        write_test_entry(&*logger, "a2", "2026-01-01T00:01:00Z");
        let tool = AuditLogTool::new(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
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
        let logger = make_test_logger();
        logger.log(&AuditLogEntry {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "a1".to_string(),
            tool_name: "file".to_string(),
            operation: "write /x".to_string(),
            reason: "approved".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Approved,
        });
        logger.log(&AuditLogEntry {
            timestamp: "2026-01-01T00:01:00Z".to_string(),
            agent_id: "a2".to_string(),
            tool_name: "command".to_string(),
            operation: format!("rm {}", std::env::temp_dir().to_str().unwrap()),
            reason: "denied".to_string(),
            risk_level: RiskLevel::High,
            session_mode: None,
            disposition: AuditDisposition::Rejected,
        });
        let tool = AuditLogTool::new(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
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
        let logger = make_test_logger();
        logger.log(&AuditLogEntry {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "early".to_string(),
            tool_name: "file".to_string(),
            operation: "w".to_string(),
            reason: "r".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Approved,
        });
        logger.log(&AuditLogEntry {
            timestamp: "2026-06-15T12:00:00Z".to_string(),
            agent_id: "mid".to_string(),
            tool_name: "file".to_string(),
            operation: "w".to_string(),
            reason: "r".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Approved,
        });
        logger.log(&AuditLogEntry {
            timestamp: "2026-12-31T23:59:59Z".to_string(),
            agent_id: "late".to_string(),
            tool_name: "file".to_string(),
            operation: "w".to_string(),
            reason: "r".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Approved,
        });
        let tool = AuditLogTool::new(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
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
    async fn test_audit_log_tool_call_invalid_since_returns_error() {
        let tool = AuditLogTool::new(make_test_logger());
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let result = tool
            .call(serde_json::json!({"since": "not-a-time"}), &ctx)
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn test_audit_log_tool_call_combined_filters() {
        let logger = make_test_logger();
        logger.log(&AuditLogEntry {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "a1".to_string(),
            tool_name: "file".to_string(),
            operation: "w".to_string(),
            reason: "r".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Approved,
        });
        logger.log(&AuditLogEntry {
            timestamp: "2026-06-01T00:00:00Z".to_string(),
            agent_id: "a1".to_string(),
            tool_name: "file".to_string(),
            operation: "w".to_string(),
            reason: "r".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Rejected,
        });
        logger.log(&AuditLogEntry {
            timestamp: "2026-06-01T00:00:00Z".to_string(),
            agent_id: "a2".to_string(),
            tool_name: "file".to_string(),
            operation: "w".to_string(),
            reason: "r".to_string(),
            risk_level: RiskLevel::Low,
            session_mode: None,
            disposition: AuditDisposition::Rejected,
        });
        let tool = AuditLogTool::new(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
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
        let logger = make_test_logger();
        write_test_entry(&*logger, "a1", "2026-01-01T00:00:00Z");
        let tool = AuditLogTool::new(logger);
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let result = tool
            .call(serde_json::json!({"agent_id": "nonexistent"}), &ctx)
            .await
            .unwrap();
        assert_eq!(result.data["total"], 0);
        assert!(result.data["entries"].as_array().unwrap().is_empty());
    }
}
