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
    ///
    /// Time bounds compare parsed absolute instants (RFC 3339) so mixed
    /// ISO 8601 forms — offsets like `+08:00` vs `Z`, differing
    /// fractional-second precision — order correctly; lexicographic
    /// comparison would misjudge them (`+` sorts before `Z`). A bound
    /// that fails to parse acts as no constraint; an entry timestamp that
    /// fails to parse cannot be compared, so it fails whenever a time
    /// bound is present. Both bounds stay inclusive.
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
        if self.since.is_some() || self.until.is_some() {
            let parse_bound = |value: &str| chrono::DateTime::parse_from_rfc3339(value).ok();
            let entry_time = match parse_bound(&entry.timestamp) {
                Some(time) => time,
                None => return false,
            };
            if let Some(bound) = self.since.as_deref().and_then(parse_bound) {
                if entry_time < bound {
                    return false;
                }
            }
            if let Some(bound) = self.until.as_deref().and_then(parse_bound) {
                if entry_time > bound {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::MemAuditLogger;

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
    /// ISO 8601 bounds compared as parsed absolute instants, each bound is
    /// open-ended when `None`, and the window ANDs with the other
    /// dimensions.
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

    /// Bounds compare parsed absolute instants: an entry written with a
    /// `+08:00` offset matches a `Z`-form window at the same instant
    /// (both bounds inclusive), and instants outside are excluded.
    #[test]
    fn test_audit_log_filter_cross_offset_bounds() {
        let window = AuditLogFilter {
            since: Some("2026-01-02T00:00:00Z".to_string()),
            until: Some("2026-01-02T00:00:00Z".to_string()),
            ..Default::default()
        };
        // Same instant as the bounds, written with a +08:00 offset.
        let same_instant = make_entry(
            "a1",
            "2026-01-02T08:00:00+08:00",
            AuditDisposition::Approved,
        );
        assert!(
            window.matches(&same_instant),
            "entry at the same instant with another offset must match both inclusive bounds"
        );
        // One second before / after the window instant, +08:00 form.
        let before = make_entry(
            "a1",
            "2026-01-02T07:59:59+08:00",
            AuditDisposition::Approved,
        );
        assert!(
            !window.matches(&before),
            "entry one second before the window must be excluded"
        );
        let after = make_entry(
            "a1",
            "2026-01-02T08:00:01+08:00",
            AuditDisposition::Approved,
        );
        assert!(
            !window.matches(&after),
            "entry one second after the window must be excluded"
        );
    }

    /// Bounds compare parsed instants, so differing fractional-second
    /// precision orders correctly (lexicographic comparison would misjudge
    /// `.` against `Z`).
    #[test]
    fn test_audit_log_filter_fractional_second_precision() {
        let window = AuditLogFilter {
            since: Some("2026-01-02T00:00:00Z".to_string()),
            until: Some("2026-01-02T00:00:01Z".to_string()),
            ..Default::default()
        };
        let mid = make_entry("a1", "2026-01-02T00:00:00.500Z", AuditDisposition::Approved);
        assert!(
            window.matches(&mid),
            "entry with fractional seconds inside the window must match"
        );
        // Exactly at `since`, written with a redundant zero fraction.
        let at_since = make_entry("a1", "2026-01-02T00:00:00.000Z", AuditDisposition::Approved);
        assert!(
            window.matches(&at_since),
            "entry exactly at `since` with a zero fraction must match (inclusive)"
        );
        // Exactly at `until` with a nanosecond zero fraction.
        let at_until = make_entry(
            "a1",
            "2026-01-02T00:00:01.000000000Z",
            AuditDisposition::Approved,
        );
        assert!(
            window.matches(&at_until),
            "entry exactly at `until` with a zero fraction must match (inclusive)"
        );
        // One nanosecond after `until`: excluded although the second field
        // reads equal to the bound.
        let just_after = make_entry(
            "a1",
            "2026-01-02T00:00:01.000000001Z",
            AuditDisposition::Approved,
        );
        assert!(
            !window.matches(&just_after),
            "entry one nanosecond after `until` must be excluded"
        );
        // The bound itself carries the fractional part instead.
        let nano_since = AuditLogFilter {
            since: Some("2026-01-02T00:00:00.000000001Z".to_string()),
            ..Default::default()
        };
        assert!(
            !nano_since.matches(&at_since),
            "entry one nanosecond before a nanosecond `since` must be excluded"
        );
    }

    /// A bound that is not valid RFC 3339 is treated as no constraint on
    /// that side (bound values are pre-validated upstream); a valid bound
    /// on the other side still applies.
    #[test]
    fn test_audit_log_filter_invalid_bound_ignored() {
        let entry = make_entry("a1", "2026-01-02T00:00:00Z", AuditDisposition::Approved);
        let bad_since = AuditLogFilter {
            since: Some("not-a-time".to_string()),
            ..Default::default()
        };
        assert!(
            bad_since.matches(&entry),
            "unparseable `since` must act as no constraint"
        );
        let bad_until = AuditLogFilter {
            until: Some("not-a-time".to_string()),
            ..Default::default()
        };
        assert!(
            bad_until.matches(&entry),
            "unparseable `until` must act as no constraint"
        );
        let mixed = AuditLogFilter {
            since: Some("not-a-time".to_string()),
            until: Some("2026-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(
            !mixed.matches(&entry),
            "the valid `until` must still exclude when `since` is unparseable"
        );
    }

    /// An entry timestamp that cannot be parsed cannot be compared to a
    /// time bound, so it fails whenever a time bound is present; without
    /// time bounds it still participates in the other dimensions.
    #[test]
    fn test_audit_log_filter_unparseable_entry_timestamp() {
        let garbage = make_entry("a1", "yesterday-ish", AuditDisposition::Approved);
        let since_only = AuditLogFilter {
            since: Some("2026-01-02T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(
            !since_only.matches(&garbage),
            "unparseable timestamp must not match a `since`-bounded filter"
        );
        let until_only = AuditLogFilter {
            until: Some("2026-01-02T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(
            !until_only.matches(&garbage),
            "unparseable timestamp must not match an `until`-bounded filter"
        );
        // Without time bounds the entry is judged on the other dimensions.
        let agent_only = AuditLogFilter {
            agent_id: Some("a1".to_string()),
            ..Default::default()
        };
        assert!(
            agent_only.matches(&garbage),
            "unparseable timestamp must not matter without time bounds"
        );
        assert!(
            AuditLogFilter::default().matches(&garbage),
            "unparseable timestamp must not matter for an empty filter"
        );
    }

    /// Combined case: an unparseable bound does not rescue an entry whose
    /// timestamp is also garbage — the mere presence of a time bound makes
    /// the entry fail (pin: unparseable-with-time-bound means no match).
    #[test]
    fn test_audit_log_filter_invalid_bound_and_garbage_timestamp_no_match() {
        let garbage = make_entry("a1", "yesterday-ish", AuditDisposition::Approved);
        let bad_since = AuditLogFilter {
            since: Some("not-a-time".to_string()),
            ..Default::default()
        };
        assert!(
            !bad_since.matches(&garbage),
            "garbage timestamp must not match even when `since` itself is unparseable"
        );
        let bad_until = AuditLogFilter {
            until: Some("not-a-time".to_string()),
            ..Default::default()
        };
        assert!(
            !bad_until.matches(&garbage),
            "garbage timestamp must not match even when `until` itself is unparseable"
        );
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
