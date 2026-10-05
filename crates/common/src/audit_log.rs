//! Audit logging trait and payloads for permission decisions.
//!
//! Provides the shared contract for recording and querying structured
//! audit entries of dangerous operations. Definitions live here so that
//! consumers (daemon, tools) can record and view audit entries without
//! depending on `closeclaw-permission`; the file-backed implementation
//! stays in the permission crate.

use crate::permission_types::RiskLevel;
use crate::session_mode::SessionMode;
use serde::{Deserialize, Serialize};

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

/// Trait for recording and querying audit log entries.
///
/// Implemented by `FileAuditLogger` (`closeclaw-permission`); consumed by
/// the permission engine and approval flow (recording) and by the tools
/// crate's `AuditLogTool` (viewing) — viewing the audit log always goes
/// through this trait's query capability.
pub trait AuditLogger: Send + Sync {
    /// Log an audit entry.
    fn log(&self, entry: &AuditLogEntry);

    /// Query audit log entries matching `filter`, newest first
    /// (reverse chronological order).
    fn query_entries(&self, filter: &AuditLogFilter) -> Vec<AuditLogEntry>;
}

/// Filter criteria for querying audit log entries.
///
/// All fields are optional; `None` means "no filter".
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
        // JSONL wire format: risk level stays lowercase.
        assert!(json.contains("\"risk_level\":\"high\""), "json={json}");
        let parsed: AuditLogEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.agent_id, "agent-1");
        assert_eq!(parsed.tool_name, "file");
        assert_eq!(parsed.disposition, AuditDisposition::Approved);
        assert_eq!(parsed.risk_level, RiskLevel::High);
        assert_eq!(parsed.session_mode, Some(SessionMode::Auto));
    }

    #[test]
    fn test_audit_log_entry_jsonl_roundtrip_lowercase_risk_level() {
        for level in [
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ] {
            let entry = AuditLogEntry {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                agent_id: "agent-1".to_string(),
                tool_name: "file".to_string(),
                operation: "write /x".to_string(),
                reason: "r".to_string(),
                risk_level: level,
                session_mode: None,
                disposition: AuditDisposition::Approved,
            };
            // One JSONL line, as written to the audit log file.
            let line = serde_json::to_string(&entry).unwrap();
            let level_json = serde_json::to_string(&level).unwrap();
            let expected_level = match level {
                RiskLevel::Low => "\"low\"",
                RiskLevel::Medium => "\"medium\"",
                RiskLevel::High => "\"high\"",
                RiskLevel::Critical => "\"critical\"",
            };
            assert_eq!(level_json, expected_level);
            assert!(
                line.contains(&format!("\"risk_level\":{level_json}")),
                "line={line}"
            );
            let parsed: AuditLogEntry = serde_json::from_str(&line).unwrap();
            assert_eq!(parsed.risk_level, level);
            assert_eq!(parsed.agent_id, "agent-1");
            assert_eq!(parsed.disposition, AuditDisposition::Approved);
            assert_eq!(parsed.session_mode, None);
            assert_eq!(parsed.timestamp, "2026-01-01T00:00:00Z");
        }
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

    /// Time-window semantics of `matches`: `since` / `until` are inclusive
    /// ISO 8601 bounds compared lexicographically, each bound is open-ended
    /// when `None`, and the window ANDs with the other dimensions.
    ///
    /// (agent_id / disposition alone, combination and empty filter are
    /// covered by `test_audit_logger_trait_log_and_query_roundtrip` and the
    /// permission crate's `FileAuditLogger` query tests.)
    #[test]
    fn test_audit_log_filter_time_bounds_inclusive_and_exclusive() {
        let entry_at = |ts: &str, agent: &str| make_entry(agent, ts, AuditDisposition::Approved);

        let window = AuditLogFilter {
            since: Some("2026-01-02T00:00:00Z".to_string()),
            until: Some("2026-01-03T00:00:00Z".to_string()),
            ..Default::default()
        };

        // Inclusive bounds: an entry exactly at since / until still matches.
        assert!(
            window.matches(&entry_at("2026-01-02T00:00:00Z", "a1")),
            "entry at `since` must match (inclusive lower bound)"
        );
        assert!(
            window.matches(&entry_at("2026-01-03T00:00:00Z", "a1")),
            "entry at `until` must match (inclusive upper bound)"
        );
        assert!(
            window.matches(&entry_at("2026-01-02T12:00:00Z", "a1")),
            "entry inside the window must match"
        );
        // Exclusive sides.
        assert!(
            !window.matches(&entry_at("2026-01-01T23:59:59Z", "a1")),
            "entry strictly before `since` must be excluded"
        );
        assert!(
            !window.matches(&entry_at("2026-01-03T00:00:01Z", "a1")),
            "entry strictly after `until` must be excluded"
        );

        // A bound left as `None` means "no filter" on that side.
        let since_only = AuditLogFilter {
            since: Some("2026-01-02T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(
            since_only.matches(&entry_at("2026-12-31T00:00:00Z", "a1")),
            "`until: None` must leave the window open-ended"
        );
        let until_only = AuditLogFilter {
            until: Some("2026-01-03T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(
            until_only.matches(&entry_at("2020-01-01T00:00:00Z", "a1")),
            "`since: None` must leave the window open-ended"
        );

        // The time window ANDs with agent_id / disposition.
        let full = AuditLogFilter {
            agent_id: Some("a1".to_string()),
            disposition: Some(AuditDisposition::Approved),
            ..window.clone()
        };
        assert!(
            full.matches(&entry_at("2026-01-02T12:00:00Z", "a1")),
            "in-window entry matching agent and disposition must match"
        );
        assert!(
            !full.matches(&entry_at("2026-01-02T12:00:00Z", "a2")),
            "in-window entry for another agent must be excluded"
        );
        let rejected = make_entry("a1", "2026-01-02T12:00:00Z", AuditDisposition::Rejected);
        assert!(
            !full.matches(&rejected),
            "in-window entry with another disposition must be excluded"
        );
        assert!(
            !full.matches(&entry_at("2026-01-04T00:00:00Z", "a1")),
            "out-of-window entry for the right agent must still be excluded"
        );
    }

    /// In-memory [`AuditLogger`] used to exercise the trait contract
    /// (record + query) through `Arc<dyn AuditLogger>`.
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
            let mut matched: Vec<AuditLogEntry> = self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|e| filter.matches(e))
                .cloned()
                .collect();
            matched.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            matched
        }
    }

    #[test]
    fn test_audit_logger_trait_log_and_query_roundtrip() {
        let logger: std::sync::Arc<dyn AuditLogger> = std::sync::Arc::new(MemAuditLogger::new());
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
            "a1",
            "2026-01-03T00:00:00Z",
            AuditDisposition::Approved,
        ));

        let all = logger.query_entries(&AuditLogFilter::default());
        assert_eq!(all.len(), 3);
        // Newest first (time-descending).
        assert_eq!(all[0].timestamp, "2026-01-03T00:00:00Z");
        assert_eq!(all[2].timestamp, "2026-01-01T00:00:00Z");

        let filtered = logger.query_entries(&AuditLogFilter {
            agent_id: Some("a1".to_string()),
            disposition: Some(AuditDisposition::Approved),
            ..Default::default()
        });
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|e| e.agent_id == "a1"));
        assert_eq!(filtered[0].timestamp, "2026-01-03T00:00:00Z");
    }
}
