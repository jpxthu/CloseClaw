//! Audit log query and export utilities

use chrono::Local;
use std::fs;
use std::path::PathBuf;

use super::AuditEvent;

/// Query filter criteria for audit logs
#[derive(Debug, Clone, Default)]
pub struct AuditQueryFilter {
    pub days: u32,
    pub event_type: Option<String>,
    pub agent: Option<String>,
    pub limit: Option<usize>,
}

/// Maximum number of days allowed in a single audit query to prevent DoS
pub const MAX_QUERY_DAYS: u32 = 365;

/// Read audit log files and filter events
pub fn query_audit_events(filter: &AuditQueryFilter) -> Vec<AuditEvent> {
    let mut results: Vec<AuditEvent> = Vec::new();
    let today = Local::now();
    // Cap days to prevent DoS via unbounded loop iteration
    let days = filter.days.min(MAX_QUERY_DAYS);
    let base_dir = {
        let home = std::env::var("HOME").ok();
        match home {
            Some(h) => PathBuf::from(h).join(".closeclaw").join("audit"),
            None => return results,
        }
    };

    let event_type_filter = filter.event_type.as_ref();
    let agent_filter = filter.agent.as_ref();

    for days_ago in 0..days {
        let date = today - chrono::Duration::days(days_ago as i64);
        let date_str = date.format("%Y-%m-%d").to_string();
        let path = base_dir.join(format!("{}.jsonl", date_str));

        if !path.exists() {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<AuditEvent>(line) {
                Ok(event) => {
                    // Filter by event_type
                    if let Some(ref et) = event_type_filter {
                        let et_str = format!("{:?}", event.event_type).to_lowercase();
                        let et_lower: String = et.to_lowercase();
                        if !et_str.contains(&et_lower) {
                            continue;
                        }
                    }
                    // Filter by agent
                    if let Some(ref ag) = agent_filter {
                        let details_str = serde_json::to_string(&event.details).unwrap_or_default();
                        let ag_str: &str = ag.as_str();
                        if !details_str.contains(ag_str) {
                            continue;
                        }
                    }
                    results.push(event);
                }
                Err(_) => continue,
            }

            // Respect limit
            if let Some(limit) = filter.limit {
                if results.len() >= limit {
                    return results;
                }
            }
        }
    }

    // Sort by timestamp descending
    results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    results
}

/// Export audit events to a file
pub fn export_audit_events(
    filter: &AuditQueryFilter,
    output_path: &str,
    format: &str,
) -> std::io::Result<usize> {
    let events = query_audit_events(filter);
    let count = events.len();

    let content = match format {
        "json" => serde_json::to_string_pretty(&events)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        "jsonl" => events
            .iter()
            .map(|e| e.serialize_to_json())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => serde_json::to_string_pretty(&events)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
    };

    fs::write(output_path, content)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditEvent, AuditEventType, AuditResult};
    use std::io::Write;
    use tempfile::TempDir;

    /// Helper: create a temp audit dir with one day's events
    fn setup_audit_dir(events: &[AuditEvent]) -> TempDir {
        let dir = TempDir::new().unwrap();
        let audit_dir = dir.path().join(".closeclaw").join("audit");
        fs::create_dir_all(&audit_dir).unwrap();

        let today = Local::now().format("%Y-%m-%d").to_string();
        let path = audit_dir.join(format!("{}.jsonl", today));
        let mut file = fs::File::create(path).unwrap();
        for event in events {
            writeln!(file, "{}", event.serialize_to_json()).unwrap();
        }
        dir
    }

    /// Run query with HOME overridden to the temp dir
    fn query_with_home(filter: &AuditQueryFilter, home: &std::path::Path) -> Vec<AuditEvent> {
        // Override HOME env var for the test
        let orig = std::env::var("HOME").ok();
        std::env::set_var("HOME", home);
        let result = query_audit_events(filter);
        if let Some(h) = orig {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
        result
    }

    #[test]
    fn test_query_returns_events() {
        let events = vec![
            AuditEvent::new(
                AuditEventType::PermissionCheck,
                serde_json::json!({"agent": "agent1"}),
                AuditResult::Allow,
            ),
        ];
        let dir = setup_audit_dir(&events);
        let filter = AuditQueryFilter {
            days: 1,
            ..Default::default()
        };
        let results = query_with_home(&filter, dir.path());
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].event_type, AuditEventType::PermissionCheck));
    }

    #[test]
    fn test_query_filter_by_event_type() {
        let events = vec![
            AuditEvent::new(
                AuditEventType::PermissionCheck,
                serde_json::json!({}),
                AuditResult::Allow,
            ),
            AuditEvent::new(
                AuditEventType::AgentStart,
                serde_json::json!({}),
                AuditResult::Allow,
            ),
        ];
        let dir = setup_audit_dir(&events);
        let filter = AuditQueryFilter {
            days: 1,
            event_type: Some("permissioncheck".to_string()),
            ..Default::default()
        };
        let results = query_with_home(&filter, dir.path());
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_query_filter_by_agent() {
        let events = vec![
            AuditEvent::new(
                AuditEventType::PermissionCheck,
                serde_json::json!({"agent": "alpha"}),
                AuditResult::Deny,
            ),
            AuditEvent::new(
                AuditEventType::PermissionCheck,
                serde_json::json!({"agent": "beta"}),
                AuditResult::Allow,
            ),
        ];
        let dir = setup_audit_dir(&events);
        let filter = AuditQueryFilter {
            days: 1,
            agent: Some("alpha".to_string()),
            ..Default::default()
        };
        let results = query_with_home(&filter, dir.path());
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_query_limit() {
        let events: Vec<AuditEvent> = (0..5)
            .map(|_| {
                AuditEvent::new(
                    AuditEventType::AgentError,
                    serde_json::json!({}),
                    AuditResult::Error,
                )
            })
            .collect();
        let dir = setup_audit_dir(&events);
        let filter = AuditQueryFilter {
            days: 1,
            limit: Some(2),
            ..Default::default()
        };
        let results = query_with_home(&filter, dir.path());
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_query_no_home_returns_empty() {
        let orig = std::env::var("HOME").ok();
        std::env::remove_var("HOME");
        let filter = AuditQueryFilter {
            days: 1,
            ..Default::default()
        };
        let results = query_audit_events(&filter);
        assert!(results.is_empty());
        if let Some(h) = orig {
            std::env::set_var("HOME", h);
        }
    }

    #[test]
    fn test_query_empty_dir() {
        let dir = TempDir::new().unwrap();
        let filter = AuditQueryFilter {
            days: 1,
            ..Default::default()
        };
        let results = query_with_home(&filter, dir.path());
        assert!(results.is_empty());
    }

    #[test]
    fn test_export_json() {
        let events = vec![
            AuditEvent::new(
                AuditEventType::ConfigReload,
                serde_json::json!({"key": "val"}),
                AuditResult::Allow,
            ),
        ];
        let dir = setup_audit_dir(&events);
        let output = dir.path().join("export.json");
        let filter = AuditQueryFilter {
            days: 1,
            ..Default::default()
        };
        let orig = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());
        let count = export_audit_events(&filter, output.to_str().unwrap(), "json").unwrap();
        if let Some(h) = orig {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
        assert_eq!(count, 1);
        let content = fs::read_to_string(&output).unwrap();
        assert!(content.contains("ConfigReload"));
    }

    #[test]
    fn test_max_query_days_cap() {
        let filter = AuditQueryFilter {
            days: 9999,
            ..Default::default()
        };
        // Should not panic even with large days value
        let _ = query_audit_events(&filter);
    }
}
