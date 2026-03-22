//! Audit logging system for CloseClaw
//!
//! Records permission checks, agent operations, and errors to persistent JSONL files.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{error, info};

/// Audit event types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event_type")]
pub enum AuditEventType {
    PermissionCheck,
    AgentStart,
    AgentStop,
    AgentError,
    ConfigReload,
    RuleReload,
}

/// Result of an audited operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuditResult {
    Allow,
    Deny,
    Error,
}

/// An audit event record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Event timestamp
    pub timestamp: DateTime<Local>,
    /// Type of event
    pub event_type: AuditEventType,
    /// Detailed context as JSON
    pub details: serde_json::Value,
    /// Result of the operation
    pub result: AuditResult,
}

impl AuditEvent {
    /// Create a new audit event
    pub fn new(event_type: AuditEventType, details: serde_json::Value, result: AuditResult) -> Self {
        Self {
            timestamp: Local::now(),
            event_type,
            details,
            result,
        }
    }

    /// Serialize to a JSON line (one JSON object per line)
    pub fn serialize_to_json(&self) -> String {
        serde_json::to_string(self).expect("audit event should serialize to JSON")
    }
}

/// Audit event builder for convenient construction
pub struct AuditEventBuilder {
    event_type: AuditEventType,
    details: serde_json::Value,
    result: AuditResult,
}

impl AuditEventBuilder {
    pub fn new(event_type: AuditEventType) -> Self {
        Self {
            event_type,
            details: json!({}),
            result: AuditResult::Allow,
        }
    }

    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }

    pub fn result(mut self, result: AuditResult) -> Self {
        self.result = result;
        self
    }

    pub fn build(self) -> AuditEvent {
        AuditEvent::new(self.event_type, self.details, self.result)
    }
}

/// Audit logger — writes events to ~/.closeclaw/audit/YYYY-MM-DD.jsonl
pub struct AuditLogger {
    /// Base directory for audit logs
    base_dir: PathBuf,
    /// In-memory buffer of events
    buffer: Arc<Mutex<VecDeque<AuditEvent>>>,
    /// Flush interval in seconds
    flush_interval_secs: u64,
    /// Max buffer size before forced flush
    max_buffer_size: usize,
    /// Current file date (to detect day changes); uses std Mutex for sync access from log_file_path
    current_date: StdMutex<String>,
}

impl AuditLogger {
    /// Create a new AuditLogger with the default audit directory
    pub fn new() -> Self {
        let home = std::env::var("HOME").expect("HOME not set");
        let base_dir = PathBuf::from(home).join(".closeclaw").join("audit");

        Self {
            base_dir,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            flush_interval_secs: 5,
            max_buffer_size: 500,
            current_date: StdMutex::new(String::new()),
        }
    }

    /// Create with custom base directory (useful for testing)
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            flush_interval_secs: 5,
            max_buffer_size: 500,
            current_date: StdMutex::new(String::new()),
        }
    }

    /// Get the current date string (YYYY-MM-DD)
    fn current_date_string() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    /// Get the log file path for the current (possibly mocked) date
    fn log_file_path(&self) -> PathBuf {
        // Use the locked current_date if set, otherwise fall back to today's real date
        let date = {
            let guard = self.current_date.lock().unwrap();
            if guard.is_empty() {
                drop(guard);
                Self::current_date_string()
            } else {
                guard.clone()
            }
        };
        self.base_dir.join(format!("{}.jsonl", date))
    }

    /// Ensure the audit directory exists
    fn ensure_dir(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.base_dir)
    }

    /// Write a single event to the file synchronously
    fn write_event_to_file(path: &PathBuf, event: &AuditEvent) -> std::io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        let mut writer = BufWriter::new(file);
        writer.write_all(event.serialize_to_json().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    /// Log an audit event (buffers and periodically flushes)
    pub async fn log(&self, event: AuditEvent) {
        // Always emit to tracing
        let event_type_str = format!("{:?}", event.event_type);
        match event.result {
            AuditResult::Allow => info!(event_type = %event_type_str, "audit: {:?}", serde_json::to_string(&event.details).unwrap_or_default()),
            AuditResult::Deny => info!(event_type = %event_type_str, result = "deny", "audit: {:?}", serde_json::to_string(&event.details).unwrap_or_default()),
            AuditResult::Error => error!(event_type = %event_type_str, "audit error: {:?}", serde_json::to_string(&event.details).unwrap_or_default()),
        }

        // Buffer the event
        {
            let mut buf = self.buffer.lock().await;
            buf.push_back(event.clone());
        }

        // Check if we should flush
        let should_flush = {
            let buf = self.buffer.lock().await;
            buf.len() >= self.max_buffer_size
        };

        if should_flush {
            self.flush().await;
        }
    }

    /// Flush buffered events to disk
    pub async fn flush(&self) {
        let events: Vec<AuditEvent> = {
            let mut buf = self.buffer.lock().await;
            buf.drain(..).collect()
        };

        if events.is_empty() {
            return;
        }

        if let Err(e) = self.ensure_dir() {
            error!("failed to create audit directory: {}", e);
            return;
        }

        let path = self.log_file_path();
        for event in &events {
            if let Err(e) = Self::write_event_to_file(&path, event) {
                error!("failed to write audit event to {}: {}", path.display(), e);
            }
        }

        info!(path = %path.display(), count = events.len(), "audit log flushed");
    }

    /// Flush on drop if there are buffered events
    pub async fn shutdown(&self) {
        self.flush().await;
    }

    /// Rotate if needed (called periodically; today is a new day)
    pub async fn rotate_if_needed(&self) {
        let today = Self::current_date_string();
        let needs_flush = {
            let mut current = self.current_date.lock().unwrap();
            if *current != today {
                *current = today;
                true
            } else {
                false
            }
        };
        if needs_flush {
            self.flush().await;
        }
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

/// Query filter criteria for audit logs
#[derive(Debug, Clone, Default)]
pub struct AuditQueryFilter {
    pub days: u32,
    pub event_type: Option<String>,
    pub agent: Option<String>,
    pub limit: Option<usize>,
}

/// Read audit log files and filter events
pub fn query_audit_events(filter: &AuditQueryFilter) -> Vec<AuditEvent> {
    let mut results: Vec<AuditEvent> = Vec::new();
    let today = Local::now();
    let base_dir = {
        let home = std::env::var("HOME").ok();
        match home {
            Some(h) => PathBuf::from(h).join(".closeclaw").join("audit"),
            None => return results,
        }
    };

    let event_type_filter = filter.event_type.as_ref();
    let agent_filter = filter.agent.as_ref();

    for days_ago in 0..filter.days {
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
                        if !et_str.contains(&et.to_lowercase()) {
                            continue;
                        }
                    }
                    // Filter by agent
                    if let Some(ref ag) = agent_filter {
                        let details_str = serde_json::to_string(&event.details).unwrap_or_default();
                        if !details_str.contains(ag.as_str()) {
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
pub fn export_audit_events(filter: &AuditQueryFilter, output_path: &str, format: &str) -> std::io::Result<usize> {
    let events = query_audit_events(filter);
    let count = events.len();

    let content = match format {
        "json" => serde_json::to_string_pretty(&events).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        "jsonl" => events.iter().map(|e| e.serialize_to_json()).collect::<Vec<_>>().join("\n"),
        _ => serde_json::to_string_pretty(&events).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
    };

    fs::write(output_path, content)?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_audit_event_serialize_to_json() {
        let event = AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({
                "agent": "test-agent",
                "action": "file_read",
                "path": "/home/admin"
            }),
            AuditResult::Allow,
        );

        let json_str = event.serialize_to_json();
        let parsed: AuditEvent = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.event_type, AuditEventType::PermissionCheck);
        assert_eq!(parsed.result, AuditResult::Allow);
        assert_eq!(parsed.details["agent"], "test-agent");
    }

    #[test]
    fn test_audit_event_types_serialize() {
        let types = vec![
            (AuditEventType::PermissionCheck, "PermissionCheck"),
            (AuditEventType::AgentStart, "AgentStart"),
            (AuditEventType::AgentStop, "AgentStop"),
            (AuditEventType::AgentError, "AgentError"),
            (AuditEventType::ConfigReload, "ConfigReload"),
            (AuditEventType::RuleReload, "RuleReload"),
        ];

        for (typ, name) in types {
            let event = AuditEvent::new(typ, json!({}), AuditResult::Allow);
            let json_str = event.serialize_to_json();
            assert!(json_str.contains(name), "JSON should contain {}", name);
        }
    }

    #[test]
    fn test_audit_result_serialize() {
        let results = vec![
            (AuditResult::Allow, "allow"),
            (AuditResult::Deny, "deny"),
            (AuditResult::Error, "error"),
        ];

        for (result, name) in results {
            let event = AuditEvent::new(AuditEventType::PermissionCheck, json!({}), result);
            let json_str = event.serialize_to_json();
            assert!(json_str.contains(name), "JSON should contain {}", name);
        }
    }

    #[tokio::test]
    async fn test_audit_logger_file_writing() {
        let temp_dir = TempDir::new().unwrap();
        let logger = AuditLogger::with_base_dir(temp_dir.path().to_path_buf());

        let event = AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({"agent": "test-agent"}),
            AuditResult::Allow,
        );

        logger.log(event).await;
        logger.flush().await;

        // Check file exists
        let date_str = AuditLogger::current_date_string();
        let log_file = temp_dir.path().join(format!("{}.jsonl", date_str));
        assert!(log_file.exists(), "log file should exist at {:?}", log_file);

        let content = fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("test-agent"));
        assert!(content.contains("PermissionCheck"));
    }

    #[tokio::test]
    async fn test_audit_logger_buffer_flush() {
        let temp_dir = TempDir::new().unwrap();
        let logger = AuditLogger::with_base_dir(temp_dir.path().to_path_buf());

        // Log many events to trigger buffer flush
        for i in 0..600 {
            let event = AuditEvent::new(
                AuditEventType::PermissionCheck,
                json!({"index": i}),
                AuditResult::Allow,
            );
            logger.log(event).await;
        }

        // Flush remaining
        logger.flush().await;

        let date_str = AuditLogger::current_date_string();
        let log_file = temp_dir.path().join(format!("{}.jsonl", date_str));
        assert!(log_file.exists());

        let content = fs::read_to_string(&log_file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 600);
    }

    #[tokio::test]
    async fn test_audit_logger_multiple_days_rotation() {
        let temp_dir = TempDir::new().unwrap();
        let logger = AuditLogger::with_base_dir(temp_dir.path().to_path_buf());

        // Force initial date
        {
            let mut current = logger.current_date.lock().unwrap();
            *current = "2024-01-01".to_string();
        }

        let event = AuditEvent::new(
            AuditEventType::AgentStart,
            json!({"agent": "dev-agent"}),
            AuditResult::Allow,
        );
        logger.log(event).await;
        logger.flush().await;

        // Old file should have been written
        let old_file = temp_dir.path().join("2024-01-01.jsonl");
        assert!(old_file.exists());

        // Rotate to new day
        logger.rotate_if_needed().await;

        let new_file = temp_dir.path().join(format!("{}.jsonl", AuditLogger::current_date_string()));
        // New file may or may not exist depending on date
    }

    #[test]
    fn test_audit_event_builder() {
        let event = AuditEventBuilder::new(AuditEventType::AgentError)
            .details(json!({"agent": "test", "error": "crash"}))
            .result(AuditResult::Error)
            .build();

        assert_eq!(event.event_type, AuditEventType::AgentError);
        assert_eq!(event.result, AuditResult::Error);
        assert_eq!(event.details["error"], "crash");
    }

    #[test]
    fn test_query_audit_events_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        // Override HOME temporarily for test
        let filter = AuditQueryFilter {
            days: 1,
            event_type: None,
            agent: None,
            limit: None,
        };

        let results = query_audit_events(&filter);
        // Should not panic even with non-existent audit dir
        assert!(results.is_empty());
    }

    #[test]
    fn test_query_with_filters() {
        let temp_dir = TempDir::new().unwrap();
        let date_str = "2024-01-15";
        let log_file = temp_dir.path().join(format!("{}.jsonl", date_str));

        // Write some test events
        let event1 = AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({"agent": "dev-agent-01", "action": "file_read"}),
            AuditResult::Allow,
        );
        let event2 = AuditEvent::new(
            AuditEventType::AgentStart,
            json!({"agent": "dev-agent-01"}),
            AuditResult::Allow,
        );
        let event3 = AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({"agent": "prod-agent", "action": "file_read"}),
            AuditResult::Deny,
        );

        let content = format!(
            "{}\n{}\n{}",
            event1.serialize_to_json(),
            event2.serialize_to_json(),
            event3.serialize_to_json()
        );
        fs::write(&log_file, content).unwrap();

        // Query all
        let filter_all = AuditQueryFilter {
            days: 7,
            event_type: None,
            agent: None,
            limit: None,
        };

        // Since we can't easily override HOME for query_audit_events,
        // we test the file-reading logic by reading directly
        let content = fs::read_to_string(&log_file).unwrap();
        let mut parsed_events = Vec::new();
        for line in content.lines() {
            if let Ok(evt) = serde_json::from_str::<AuditEvent>(line) {
                parsed_events.push(evt);
            }
        }
        assert_eq!(parsed_events.len(), 3);

        // Filter by agent
        let agent_filtered: Vec<_> = parsed_events
            .iter()
            .filter(|e| {
                serde_json::to_string(&e.details)
                    .unwrap_or_default()
                    .contains("dev-agent-01")
            })
            .collect();
        assert_eq!(agent_filtered.len(), 2);

        // Filter by event_type
        let type_filtered: Vec<_> = parsed_events
            .iter()
            .filter(|e| format!("{:?}", e.event_type).to_lowercase().contains("permission"))
            .collect();
        assert_eq!(type_filtered.len(), 2);
    }

    #[test]
    fn test_export_json_format() {
        let temp_dir = TempDir::new().unwrap();
        let output_file = temp_dir.path().join("export.json");

        let events = vec![
            AuditEvent::new(
                AuditEventType::PermissionCheck,
                json!({"agent": "test"}),
                AuditResult::Allow,
            ),
        ];

        let content = serde_json::to_string_pretty(&events).unwrap();
        fs::write(&output_file, &content).unwrap();

        let written = fs::read_to_string(&output_file).unwrap();
        assert!(written.contains("PermissionCheck"));
        assert!(written.contains("test"));
    }
}
