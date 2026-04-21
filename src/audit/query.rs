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
